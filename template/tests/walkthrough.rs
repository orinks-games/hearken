use hearken_app::{Headless, Key};
use hello_hearken::{HelloGame, Title};

fn start() -> Headless<HelloGame> {
    Headless::new((), Title::new())
}

#[test]
fn the_title_announces_itself() {
    let h = start();
    assert_eq!(h.spoken(), ["Hello Hearken.", "Play. 1 of 4."]);
}

#[test]
fn settings_open_and_the_title_comes_back_where_it_was() {
    let mut h = start();
    h.press(Key::Down);
    h.clear();
    h.press(Key::Return);
    assert_eq!(
        h.spoken(),
        ["Settings.", "Speech rate: 50 percent. 1 of 2."]
    );
    h.clear();
    h.press(Key::Escape);
    assert_eq!(h.spoken(), ["Hello Hearken.", "Settings. 2 of 4."]);
}

#[test]
fn practice_counts_and_pauses_and_returns_to_the_title() {
    let mut h = start();
    h.press(Key::Return);
    assert_eq!(h.app.top_name(), "practice");
    h.clear();
    h.press(Key::Space);
    h.press(Key::Space);
    assert_eq!(h.spoken(), ["1", "2"]);
    h.press(Key::Escape);
    assert_eq!(h.app.top_name(), "pause");
    for _ in 0..3 {
        h.press(Key::Down);
    }
    h.press(Key::Return);
    assert_eq!((h.app.top_name(), h.app.depth()), ("title", 1));
}

#[test]
fn escape_on_the_title_says_how_to_leave_and_quit_leaves() {
    let mut h = start();
    h.clear();
    h.press(Key::Escape);
    assert_eq!(h.spoken(), ["Press Enter on Quit to leave."]);
    assert!(h.app.ctx.running);
    h.press(Key::End);
    h.press(Key::Return);
    assert!(!h.app.ctx.running);
}
