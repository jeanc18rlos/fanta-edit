//! A template built and filled in through the public API alone: a text
//! variable, a placeholder image, a role that limits the edits, change events
//! and undo, checked through exported pixels.

use fanta_api::{
    ApiError, DesignSession, ExportEncoding, ExportSizing, NodeEvent, NodeEventKind, NodeFilter,
    NodeId, NodeKind, Placeholder, Role, Scope,
};

fn png(width: u32, height: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(width, height, image::Rgba(rgba))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode png");
    bytes
}

fn render(session: &DesignSession, node: NodeId) -> Vec<u8> {
    session
        .export(node, ExportEncoding::Png, ExportSizing::Scale(1.0))
        .expect("export")
        .bytes
}

#[test]
fn a_template_is_built_and_filled_in() {
    let mut session = DesignSession::new().expect("session");
    let page = session.active_page().expect("page");

    let card = session.create_node(page, NodeKind::Frame).expect("card");
    session.set_float(card, "size/width", 240.0).expect("width");
    session
        .set_float(card, "size/height", 160.0)
        .expect("height");
    let greeting = session
        .create_node(card, NodeKind::Text("Hello {{name}}".into()))
        .expect("greeting");
    let photo = session
        .create_node(card, NodeKind::Image(png(40, 40, [0, 0, 255, 255])))
        .expect("photo");
    session
        .set_float(photo, "position/x", 180.0)
        .expect("place the photo");
    session
        .set_placeholder(
            photo,
            Some(Placeholder {
                show_overlay: true,
                show_button: true,
            }),
        )
        .expect("placeholder");
    for node in [greeting, photo] {
        session
            .set_scope_enabled(node, Scope::LayerMove, false)
            .expect("pin in place");
    }
    session.set_text_variable("name", "Ada").expect("name");
    session.add_undo_step("Template").expect("commit");
    let template = render(&session, card);

    session.set_role(Role::Adopter);
    let subscription = session.subscribe(NodeFilter::All);
    assert!(matches!(
        session.set_float(photo, "position/x", 0.0),
        Err(ApiError::NotAllowed {
            scope: Scope::LayerMove,
            ..
        })
    ));
    session
        .set_text_variable("name", "Grace Hopper")
        .expect("fill in the name");
    let renamed = render(&session, card);
    assert_ne!(renamed, template, "the variable's new value is drawn");
    session
        .replace_placeholder_image(photo, png(40, 40, [255, 0, 0, 255]))
        .expect("fill in the photo");
    let events = session.poll_events(subscription).expect("poll");
    for node in [greeting, photo] {
        assert!(
            events.contains(&NodeEvent {
                node,
                kind: NodeEventKind::Updated
            }),
            "{events:?}"
        );
    }
    session.add_undo_step("Fill in").expect("commit");
    let filled = render(&session, card);
    assert_ne!(filled, renamed, "the new photo is drawn");

    assert!(session.undo().expect("undo"));
    assert_eq!(
        render(&session, card),
        template,
        "undo restores the template"
    );
    assert!(session.redo().expect("redo"));
    assert_eq!(render(&session, card), filled);
}
