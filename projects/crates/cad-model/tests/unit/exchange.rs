use super::*;

#[test]
fn light_default_layer_colour_is_darkened_for_export() {
    let original = color_of("#d0d7de");
    let dark = export_color(original);
    assert!(luma(original) > LIGHT_INK_LIMIT, "the viewport default must be light");
    assert!(luma(dark) < 0.5, "`{}` is not legible on white", dark.to_css());
}

#[test]
fn white_is_darkened_for_export() {
    let dark = export_color(Color::WHITE);
    assert!(luma(dark) < 0.5, "`{}` is not legible on white", dark.to_css());
}

#[test]
fn dark_layer_colours_are_kept_untouched() {
    let teal = color_of("#0b7285");
    assert_eq!(export_color(teal), teal);
    assert_eq!(export_color(Color::BLACK), Color::BLACK);
}

#[test]
fn lightened_colour_keeps_its_channel_order() {
    let dark = export_color(color_of("#d0d7de"));
    assert!(dark.blue() >= dark.green() && dark.green() >= dark.red(), "`{}` lost the cool grey hue", dark.to_css());
}
