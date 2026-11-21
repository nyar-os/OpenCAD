//! Process-wide drawing store.
//!
//! Handles stay stable for the lifetime of the process so the existing
//! `create`/`open`/`save`/`inspect`/`export`/`close` signatures keep working.
//! The store holds one [`Session`] per handle and never shares a document
//! between handles.
//!
//! Every session keeps a bounded history of the drawings its mutating calls
//! replaced. `undo` and `redo` swap the current document with one of those
//! snapshots and bump the revision; saves and exports are not history steps.

use std::{
    collections::HashMap,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use boost_session::History;
use serde_json::{Value, json};

use crate::{ServiceError, schema::Drawing};

/// Maximum number of snapshots one handle keeps.
pub(crate) const HISTORY_LIMIT: usize = 32;

/// One open drawing plus its undo history.
pub(crate) struct Session {
    /// Current committed document.
    pub(crate) drawing: Drawing,
    history: History<Drawing>,
}

impl Session {
    fn new(drawing: Drawing) -> Self {
        Self { drawing, history: History::new(HISTORY_LIMIT) }
    }

    /// Summary of this session, including whether undo and redo can run.
    pub(crate) fn summary(&self) -> Value {
        let mut summary = self.drawing.summary();
        summary["canUndo"] = json!(self.history.can_undo());
        summary["canRedo"] = json!(self.history.can_redo());
        summary
    }
}

fn sessions() -> &'static Mutex<HashMap<u64, Session>> {
    static SESSIONS: OnceLock<Mutex<HashMap<u64, Session>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::SeqCst)
}

fn invalid_handle(handle: u64) -> ServiceError {
    ServiceError::new("drawing/not-found", format!("no drawing is open for handle {handle}"))
}

/// Inserts a document with a fresh, empty history and returns its new handle.
pub(crate) fn insert(drawing: Drawing) -> u64 {
    let handle = next_handle();
    let mut guard = sessions().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.insert(handle, Session::new(drawing));
    handle
}

/// Reads one session behind a closure.
pub(crate) fn read<T>(handle: u64, work: impl FnOnce(&Session) -> T) -> Result<T, ServiceError> {
    let guard = sessions().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let session = guard.get(&handle).ok_or_else(|| invalid_handle(handle))?;
    Ok(work(session))
}

/// Mutates one document and records the previous drawing as an undo step.
///
/// A failed operation leaves the document and the history untouched, so an
/// error can never be undone into a different state.
pub(crate) fn edit<T>(handle: u64, work: impl FnOnce(&mut Drawing) -> Result<T, ServiceError>) -> Result<T, ServiceError> {
    let mut guard = sessions().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let session = guard.get_mut(&handle).ok_or_else(|| invalid_handle(handle))?;
    let before = session.drawing.clone();
    match work(&mut session.drawing) {
        Ok(value) => {
            session.history.record(before);
            Ok(value)
        }
        Err(error) => {
            session.drawing = before;
            Err(error)
        }
    }
}

/// Mutates one document without adding an undo step (save and export).
pub(crate) fn persist<T>(handle: u64, work: impl FnOnce(&mut Drawing) -> Result<T, ServiceError>) -> Result<T, ServiceError> {
    let mut guard = sessions().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let session = guard.get_mut(&handle).ok_or_else(|| invalid_handle(handle))?;
    work(&mut session.drawing)
}

/// Restores the previous snapshot and bumps the revision.
pub(crate) fn undo(handle: u64) -> Result<(), ServiceError> {
    let mut guard = sessions().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let session = guard.get_mut(&handle).ok_or_else(|| invalid_handle(handle))?;
    if !session.history.can_undo() {
        return Err(ServiceError::new("drawing/nothing-to-undo", "the drawing has no earlier state to restore"));
    }
    let current = session.drawing.clone();
    let Some(previous) = session.history.undo(current)
    else {
        return Err(ServiceError::new("drawing/nothing-to-undo", "the drawing has no earlier state to restore"));
    };
    let revision = session.drawing.revision;
    let next_entity = session.drawing.next_entity;
    session.drawing = previous;
    session.drawing.revision = revision + 1;
    session.drawing.next_entity = session.drawing.next_entity.max(next_entity);
    Ok(())
}

/// Restores the snapshot that {@link undo} replaced and bumps the revision.
pub(crate) fn redo(handle: u64) -> Result<(), ServiceError> {
    let mut guard = sessions().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let session = guard.get_mut(&handle).ok_or_else(|| invalid_handle(handle))?;
    if !session.history.can_redo() {
        return Err(ServiceError::new("drawing/nothing-to-redo", "the drawing has no undone step to reapply"));
    }
    let current = session.drawing.clone();
    let Some(next) = session.history.redo(current)
    else {
        return Err(ServiceError::new("drawing/nothing-to-redo", "the drawing has no undone step to reapply"));
    };
    let revision = session.drawing.revision;
    let next_entity = session.drawing.next_entity;
    session.drawing = next;
    session.drawing.revision = revision + 1;
    session.drawing.next_entity = session.drawing.next_entity.max(next_entity);
    Ok(())
}

/// Drops one document.
pub(crate) fn remove(handle: u64) -> Result<(), ServiceError> {
    let mut guard = sessions().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.remove(&handle).map(|_| ()).ok_or_else(|| invalid_handle(handle))
}
