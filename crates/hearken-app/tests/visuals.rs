#![cfg(feature = "visuals")]
mod common;

use common::{Probe, TestData, TestGame};
use hearken_app::visuals::{Renderer, Scene};
use hearken_app::{Ctx, Headless, State};

struct World;
impl State<TestGame> for World {
    fn name(&self) -> &'static str {
        "world"
    }
    fn help(&self, _: &Ctx<TestGame>) -> String {
        "Help".into()
    }
    fn lines(&self, _: &Ctx<TestGame>) -> Vec<String> {
        vec!["World".into()]
    }
    fn visuals(&self, ctx: &Ctx<TestGame>) -> Scene {
        let mut scene = Scene::from_lines(self.lines(ctx));
        scene.circle(100., 100., 30., [255, 255, 255]);
        scene
    }
}

#[test]
fn captures_do_not_speak_or_advance_and_covering_hides_the_world() {
    let mut h = Headless::new(TestData::default(), World);
    h.app.ctx.say("A caption");
    let clock = h.app.ctx.clock;
    let spoken = h.spoken();
    let a = h.app.visual_scene();
    assert_eq!(a, h.app.visual_scene());
    assert_eq!(a.caption, "A caption");
    assert!(!a.shapes.is_empty());
    assert_eq!(h.app.ctx.clock, clock);
    assert_eq!(h.spoken(), spoken);
    h.app.ctx.push(Probe::new("covered"));
    h.frame(0.);
    assert!(h.app.visual_scene().shapes.is_empty());
}

#[test]
fn unchanged_frames_are_skipped_but_resize_and_exposure_redraw() {
    let scene = Scene::from_lines(vec!["Menu".into(), "> Play".into(), "  Quit".into()]);
    let mut r = Renderer::default();
    assert!(r.build(&scene, (1280, 800), false));
    assert!(!r.build(&scene, (1280, 800), false));
    assert!(r.build(&scene, (960, 640), false));
    assert!(r.build(&scene, (960, 640), true));
    assert!(r.painter.vertex_count() > 0);
}

#[test]
fn software_render_draws_text_and_focus_without_a_window() {
    use sdl2::pixels::PixelFormatEnum;
    let surface = sdl2::surface::Surface::new(960, 640, PixelFormatEnum::RGBA32).unwrap();
    let mut canvas = surface.into_canvas().unwrap();
    let mut atlas = canvas
        .texture_creator()
        .create_texture_streaming(PixelFormatEnum::RGBA32, 1024, 1024)
        .unwrap();
    let mut renderer = Renderer::default();
    let scene = Scene::from_lines(vec!["Menu".into(), "> Play".into(), "  Quit".into()]);
    renderer.build(&scene, (960, 640), true);
    renderer.draw(&mut canvas, &mut atlas).unwrap();
    let pixels = canvas.read_pixels(None, PixelFormatEnum::RGBA32).unwrap();
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 200 && p[1] > 150)
            .count()
            > 1000
    );
}

#[test]
fn scrolling_reaches_long_text_and_caption_history() {
    let mut scene = Scene::from_lines((0..100).map(|i| format!("Screen line {i}")).collect());
    scene.captions = (0..100).map(|i| format!("Spoken line {i}")).collect();
    scene.text_percent = 200;
    let mut r = Renderer::default();
    r.build(&scene, (960, 640), true);
    let before = r.painter.batches[2].vertices.clone();
    r.scroll(30, false);
    r.scroll(-30, true);
    assert!(r.build(&scene, (960, 640), true));
    assert_ne!(
        format!("{before:?}"),
        format!("{:?}", r.painter.batches[2].vertices)
    );
}
