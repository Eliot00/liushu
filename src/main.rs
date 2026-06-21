mod composor;
mod keyboard;

use composor::Composor;
use keyboard::KeyboardProcessorResponse;
use liushu_core::engine::Engine;
use liushu_core::engine::candidates::Candidate;
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    protocol::{wl_keyboard, wl_registry},
};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2, zwp_input_method_v2, zwp_input_method_keyboard_grab_v2,
};
use xdg::BaseDirectories;

fn main() {
    let conn = Connection::connect_to_env().unwrap();
    let mut event_queue = conn.new_event_queue();
    let qhandle = event_queue.handle();

    let display = conn.display();
    display.get_registry(&qhandle, ());

    let xdg_dirs = BaseDirectories::with_prefix("liushu").unwrap();
    let dict_path = xdg_dirs.find_data_file("sunman.trie").unwrap();
    let engine = Engine::new(dict_path).expect("Open dict error");
    let composor = Composor::with_engine(engine);
    let mut state = AppState {
        running: true,
        composor,
        ..Default::default()
    };

    while state.running {
        event_queue.blocking_dispatch(&mut state).unwrap();
    }
}

#[derive(Default)]
struct AppState {
    running: bool,
    input: String,
    candidates: Vec<Candidate>,
    composor: Composor,
    keyboard_processor: keyboard::KeyboardProcessor,
    is_ascii_mode: bool,
    // v2 objects
    input_method_manager: Option<zwp_input_method_manager_v2::ZwpInputMethodManagerV2>,
    input_method: Option<zwp_input_method_v2::ZwpInputMethodV2>,
    keyboard_grab: Option<zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2>,
    seat: Option<WlSeat>,
    /// The serial that must be passed to `commit()` — equals the number of
    /// `done` events received so far from the compositor.
    done_serial: u32,
}

// ---------------------------------------------------------------------------
// Registry — bind the v2 manager and the wl_seat
// ---------------------------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: <wl_registry::WlRegistry as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            match &interface[..] {
                "zwp_input_method_manager_v2" => {
                    let mgr = registry
                        .bind::<zwp_input_method_manager_v2::ZwpInputMethodManagerV2, _, _>(
                            name, 1, qh, (),
                        );
                    state.input_method_manager = Some(mgr);
                }
                "wl_seat" => {
                    let seat = registry.bind::<WlSeat, _, _>(name, 7, qh, ());
                    state.seat = Some(seat);
                }
                _ => {}
            }

            // Once we have both manager and seat, create the input method.
            if state.input_method.is_none() {
                if let (Some(mgr), Some(seat)) =
                    (state.input_method_manager.as_ref(), state.seat.as_ref())
                {
                    let im = mgr.get_input_method(seat, qh, ());
                    state.input_method = Some(im);
                }
            }
        }
    }
}

// Required stub Dispatch for the manager (no events on manager objects).
impl Dispatch<zwp_input_method_manager_v2::ZwpInputMethodManagerV2, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
        _event: <zwp_input_method_manager_v2::ZwpInputMethodManagerV2 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

// Required stub Dispatch for the wl_seat (we don't use seat events directly).
impl Dispatch<WlSeat, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &WlSeat,
        _event: <WlSeat as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

// ---------------------------------------------------------------------------
// zwp_input_method_v2 — activate / deactivate / done / unavailable
// ---------------------------------------------------------------------------

impl Dispatch<zwp_input_method_v2::ZwpInputMethodV2, ()> for AppState {
    fn event(
        state: &mut Self,
        proxy: &zwp_input_method_v2::ZwpInputMethodV2,
        event: <zwp_input_method_v2::ZwpInputMethodV2 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        use zwp_input_method_v2::Event;
        match event {
            Event::Activate => {
                println!("method activate");
                let grab = proxy.grab_keyboard(qh, ());
                state.keyboard_grab = Some(grab);
                println!("grab keyboard");
            }
            Event::Deactivate => {
                println!("method deactivate");
                state.input.clear();
                state.composor.clear();
                if let Some(grab) = state.keyboard_grab.take() {
                    grab.release();
                }
            }
            Event::Done => {
                // Increment the serial for our next commit() call.
                state.done_serial += 1;
            }
            Event::Unavailable => {
                println!("input method unavailable");
                state.running = false;
            }
            Event::SurroundingText { text, cursor, anchor } => {
                println!("surrounding_text: \"{}\" cursor={} anchor={}", text, cursor, anchor);
            }
            Event::ContentType { hint, purpose } => {
                println!("content_type: hint={:?} purpose={:?}", hint, purpose);
            }
            Event::TextChangeCause { cause } => {
                println!("text_change_cause: {:?}", cause);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// zwp_input_method_keyboard_grab_v2 — raw key events
// ---------------------------------------------------------------------------

impl Dispatch<zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2,
        event: <zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use zwp_input_method_keyboard_grab_v2::Event;
        match event {
            Event::Key {
                key, state: keystate, ..
            } => {
                let pressed = matches!(keystate, WEnum::Value(wl_keyboard::KeyState::Pressed));

                let response = state.keyboard_processor.handle_key(
                    key, pressed, state.is_ascii_mode,
                );

                let im = match state.input_method.as_ref() {
                    Some(im) => im,
                    None => return,
                };

                match state.composor.process(response) {
                    KeyboardProcessorResponse::Result(input, candidates) => {
                        state.input = input.clone();
                        state.candidates = candidates;
                        let len = state.input.len() as i32;
                        im.set_preedit_string(state.input.clone(), 0, len);
                        im.commit(state.done_serial);
                    }
                    KeyboardProcessorResponse::Commit => {
                        let text: String = if state.input.is_empty() {
                            " ".to_owned()
                        } else if !state.candidates.is_empty() {
                            state.candidates[0].text.clone()
                        } else {
                            return;
                        };
                        im.commit_string(text);
                        im.commit(state.done_serial);
                        state.input.clear();
                        state.composor.clear();
                    }
                    KeyboardProcessorResponse::DirectlyCommit => {
                        im.commit_string(state.input.clone());
                        im.commit(state.done_serial);
                        state.input.clear();
                        state.composor.clear();
                    }
                    KeyboardProcessorResponse::Toggle => {
                        state.is_ascii_mode = !state.is_ascii_mode;
                    }
                    KeyboardProcessorResponse::Ignored
                    | KeyboardProcessorResponse::Composing(_)
                    | KeyboardProcessorResponse::Backspace => {
                        // Composing and Backspace are converted to Result or
                        // Ignored inside Composor::process; anything still
                        // falling through here is genuinely ignored.
                    }
                }
            }
            _ => {
                // Ignore keymap / modifiers / repeat_info for now.
            }
        }
    }
}
