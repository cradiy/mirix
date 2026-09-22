use super::*;

#[test]
fn supplied_pixels_and_alpha_reach_the_tray_at_each_size() {
    let icon = EngineIcon {
        images: [16, 24]
            .map(|size| IconImage {
                width: size,
                height: size,
                rgba: [15, 80, 200, 128].repeat((size * size) as usize),
            })
            .to_vec(),
    };
    let rendered = render(Some(&icon));
    assert_eq!(rendered.images().len(), icon.images.len());
    for (actual, expected) in rendered.images().iter().zip(&icon.images) {
        assert_eq!(actual.width, expected.width);
        assert_eq!(actual.height, expected.height);
        assert_eq!(actual.rgba.as_ref(), expected.rgba.as_slice());
    }
}

#[test]
fn missing_or_malformed_images_use_the_generic_badge() {
    let missing = render(None);
    let malformed = render(Some(&EngineIcon { images: vec![] }));
    assert_eq!(missing.images().len(), 3);
    for (actual, expected) in malformed.images().iter().zip(missing.images()) {
        assert_eq!(actual.rgba, expected.rgba);
    }
}
