/**
 * Build and release configuration for OpenCAD.
 *
 * Pure literals only: `wae` parses this file statically and never executes it.
 * Build order is fixed: native → ts → ui → skills → staging.
 *
 * The packages live under `projects/packages`; the native addon is built from
 * the `open-cad-napi` crate and staged into the per-platform packages.
 */
import { defineConfig } from '@wae/wae';

export default defineConfig({
    native: {
        crate: 'open-cad-napi',
        cdylib: 'open_cad_napi',
        targets: [
            {
                platform: 'win32',
                arch: 'x64',
                triple: 'x86_64-pc-windows-msvc',
                package: '@wae/open-cad-win32-x64',
                nodeFile: 'open-cad.win32-x64.node',
                dir: 'projects/packages/open-cad-win32-x64',
            },
            {
                platform: 'darwin',
                arch: 'x64',
                triple: 'x86_64-apple-darwin',
                package: '@wae/open-cad-darwin-x64',
                nodeFile: 'open-cad.darwin-x64.node',
                dir: 'projects/packages/open-cad-darwin-x64',
            },
            {
                platform: 'darwin',
                arch: 'arm64',
                triple: 'aarch64-apple-darwin',
                package: '@wae/open-cad-darwin-arm64',
                nodeFile: 'open-cad.darwin-arm64.node',
                dir: 'projects/packages/open-cad-darwin-arm64',
            },
            {
                platform: 'linux',
                arch: 'x64',
                triple: 'x86_64-unknown-linux-gnu',
                package: '@wae/open-cad-linux-x64',
                nodeFile: 'open-cad.linux-x64.node',
                dir: 'projects/packages/open-cad-linux-x64',
            },
            {
                platform: 'linux',
                arch: 'arm64',
                triple: 'aarch64-unknown-linux-gnu',
                package: '@wae/open-cad-linux-arm64',
                nodeFile: 'open-cad.linux-arm64.node',
                dir: 'projects/packages/open-cad-linux-arm64',
            },
        ],
    },
    packages: [
        {
            name: '@wae/open-cad-win32-x64',
            dir: 'projects/packages/open-cad-win32-x64',
            os: ['win32'],
            cpu: ['x64'],
            main: 'open-cad.win32-x64.node',
        },
        {
            name: '@wae/open-cad-darwin-x64',
            dir: 'projects/packages/open-cad-darwin-x64',
            os: ['darwin'],
            cpu: ['x64'],
            main: 'open-cad.darwin-x64.node',
        },
        {
            name: '@wae/open-cad-darwin-arm64',
            dir: 'projects/packages/open-cad-darwin-arm64',
            os: ['darwin'],
            cpu: ['arm64'],
            main: 'open-cad.darwin-arm64.node',
        },
        {
            name: '@wae/open-cad-linux-x64',
            dir: 'projects/packages/open-cad-linux-x64',
            os: ['linux'],
            cpu: ['x64'],
            main: 'open-cad.linux-x64.node',
        },
        {
            name: '@wae/open-cad-linux-arm64',
            dir: 'projects/packages/open-cad-linux-arm64',
            os: ['linux'],
            cpu: ['arm64'],
            main: 'open-cad.linux-arm64.node',
        },
    ],
    build: {
        ts: { packages: ['@wae/open-cad'] },
        ui: { package: '@wae/open-cad-ui' },
        stage: [
            {
                from: 'projects/packages/open-cad-ui/dist/static',
                to: 'projects/packages/open-cad/dist/gui',
                clean: true,
            },
        ],
        outputs: [
            'projects/packages/open-cad/dist/cli.js',
            'projects/packages/open-cad/dist/gui/index.html',
        ],
    },
    skills: {
        packages: [
            {
                dir: 'projects/packages/open-cad-skills/skills',
                packageJson: 'projects/packages/open-cad-skills/package.json',
                appPackageJson: 'projects/packages/open-cad/package.json',
                cliEntry: 'projects/packages/open-cad/dist/cli.js',
                commandName: 'open-cad',
            },
        ],
    },
    // `release` is intentionally unset while the app packages stay `private`;
    // the platform packages are the only publishable set and have no CI release
    // workflow yet.
    ci: {
        repo: 'nyar-os/OpenCAD',
        workflows: ['check-rust.yml', 'check-typescript.yml'],
        requiredJobs: ['build', 'check'],
        shaBinding: false,
    },
    install: { shortcutDefault: true },
    audit: { actor: { kind: 'flag', id: 'flag', source: 'flag' } },
    window: { mode: 'single', secondLaunch: 'focus' },
});
