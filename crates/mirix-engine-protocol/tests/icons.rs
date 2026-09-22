use std::sync::Arc;

use mirix_engine_protocol::*;

struct Method(Option<Arc<EngineIcon>>);

impl Engine for Method {
    fn name(&self) -> &str {
        "icon-fixture"
    }
    fn tray_icon(&self) -> Option<Arc<EngineIcon>> {
        self.0.clone()
    }
    fn process_key(&mut self, _: &KeyEvent) -> EngineOutput {
        EngineOutput::pass()
    }
    fn reset(&mut self) {
        self.0 = None;
    }
    fn candidates(&self) -> &[Candidate] {
        &[]
    }
    fn select(&mut self, _: CandidateId) -> EngineOutput {
        self.0 = Some(Arc::new(sample(200)));
        EngineOutput::consume()
    }
}

fn sample(red: u8) -> EngineIcon {
    EngineIcon {
        images: vec![IconImage {
            width: 1,
            height: 1,
            rgba: vec![red, 10, 20, 128],
        }],
    }
}

#[test]
fn service_sends_icons_before_replies_only_when_changed_and_can_clear_them() {
    let mut engine = Method(Some(Arc::new(sample(100))));
    let mut requests = vec![];
    for request in [
        EngineRequest::Hello,
        EngineRequest::Select { id: CandidateId(1) },
        EngineRequest::Select { id: CandidateId(2) },
        EngineRequest::Reset,
        EngineRequest::Reset,
        EngineRequest::Hello,
    ] {
        write_message(&mut requests, &request).unwrap();
    }
    let mut bytes = vec![];
    serve(&mut engine, &mut requests.as_slice(), &mut bytes).unwrap();
    let mut reader = bytes.as_slice();
    let mut next = || read_message::<_, EngineResponse>(&mut reader).unwrap();
    assert!(
        matches!(next(), EngineResponse::IconChanged { icon: Some(icon) } if icon == sample(100))
    );
    assert!(matches!(next(), EngineResponse::Ready { .. }));
    assert!(
        matches!(next(), EngineResponse::IconChanged { icon: Some(icon) } if icon == sample(200))
    );
    assert!(matches!(next(), EngineResponse::State { .. }));
    assert!(matches!(next(), EngineResponse::State { .. }));
    assert!(matches!(next(), EngineResponse::IconChanged { icon: None }));
    assert!(matches!(next(), EngineResponse::ResetDone { .. }));
    assert!(matches!(next(), EngineResponse::ResetDone { .. }));
    assert!(matches!(next(), EngineResponse::IconChanged { icon: None }));
    assert!(matches!(next(), EngineResponse::Ready { .. }));
    assert!(reader.is_empty());
}

#[test]
fn image_validation_bounds_memory_and_pixel_layout() {
    sample(1).validate().unwrap();
    let image = sample(1).images.remove(0);
    for images in [
        vec![],
        vec![image.clone(), image.clone()],
        vec![IconImage {
            width: u32::MAX,
            ..image.clone()
        }],
        vec![IconImage {
            rgba: vec![1, 2, 3],
            ..image.clone()
        }],
        vec![
            IconImage {
                width: 128,
                height: 128,
                rgba: vec![255; 65536],
            },
            image,
        ],
    ] {
        assert!(EngineIcon { images }.validate().is_err());
    }
}
