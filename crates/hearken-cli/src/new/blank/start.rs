use hearken::{Headless, Key};
use hello_hearken::{HelloGame, Start};

#[test]
fn the_start_screen_says_its_keys_and_escape_quits() {
    let mut h = Headless::<HelloGame>::new((), Start);
    assert_eq!(h.spoken(), ["Hello Hearken. Space pings, Escape quits."]);
    h.press(Key::Space);
    assert!(h.app.ctx.running);
    h.press(Key::Escape);
    assert!(!h.app.ctx.running);
}
