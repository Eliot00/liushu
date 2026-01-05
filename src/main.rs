mod composor;
mod keyboard;

use composor::Composor;
use keyboard::KeyboardProcessorResponse;
use liushu_core::engine::{Engine, candidates::Candidate};
use wayland_client::{
    Connection, Dispatch, QueueHandle, event_created_child,
    protocol::{wl_keyboard, wl_registry},
};
use wayland_protocols::wp::input_method::zv1::client::{
    zwp_input_method_context_v1,
    zwp_input_method_v1::{self, EVT_ACTIVATE_OPCODE},
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
    input_method: Option<zwp_input_method_v1::ZwpInputMethodV1>,
    context: Option<zwp_input_method_context_v1::ZwpInputMethodContextV1>,
    input_serial: u32,
    candidates: Vec<Candidate>,
    composor: Composor,
    keyboard_processor: keyboard::KeyboardProcessor,
    is_ascii_mode: bool,
}

impl AppState {
    pub fn process(&mut self, event: wl_keyboard::Event) {
        match event {
            wl_keyboard::Event::Enter { .. } => {
                println!("enter");
            }
            wl_keyboard::Event::Leave { .. } => {
                println!("leave");
            }
            wl_keyboard::Event::Key {
                serial,
                time,
                key,
                state,
            } => {
                let response = self.composor.process(
                    self.keyboard_processor
                        .handle_event(event, self.is_ascii_mode),
                );
                match (response, self.context.as_ref()) {
                    (KeyboardProcessorResponse::Commit, Some(ctx)) => {
                        if self.input.is_empty() {
                            ctx.commit_string(self.input_serial, " ".to_string());
                        } else if !self.candidates.is_empty() {
                            ctx.commit_string(self.input_serial, self.candidates[0].text.clone());
                            self.input.clear();
                            self.composor.clear();
                        }
                    }
                    (KeyboardProcessorResponse::DirectlyCommit, Some(ctx)) => {
                        ctx.commit_string(self.input_serial, self.input.clone());
                        self.input.clear();
                        self.composor.clear();
                    }
                    (KeyboardProcessorResponse::Result(input, candidates), Some(ctx)) => {
                        self.input = input;
                        self.candidates = candidates;
                        ctx.preedit_string(
                            self.input_serial,
                            self.input.clone(),
                            self.input.clone(),
                        );
                    }
                    (KeyboardProcessorResponse::Unhandled(_), Some(ctx)) => {
                        ctx.key(serial, time, key, state.into());
                    }
                    (KeyboardProcessorResponse::Toggle, _) => {
                        self.is_ascii_mode = !self.is_ascii_mode;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

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
            && &interface[..] == "zwp_input_method_v1"
        {
            let input_method =
                registry.bind::<zwp_input_method_v1::ZwpInputMethodV1, _, _>(name, 1, qh, ());
            state.input_method = Some(input_method);
        }
    }
}

impl Dispatch<zwp_input_method_v1::ZwpInputMethodV1, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &zwp_input_method_v1::ZwpInputMethodV1,
        event: zwp_input_method_v1::Event,
        _data: &(),
        _conn: &Connection,
        qhandle: &QueueHandle<Self>,
    ) {
        println!("current event is {:#?}", event);
        match event {
            zwp_input_method_v1::Event::Activate { id } => {
                println!("method activate");
                id.grab_keyboard(qhandle, ());
                state.context = Some(id);
                println!("grab keyboard");
            }
            zwp_input_method_v1::Event::Deactivate { context } => {
                state.input.clear();
                state.context = None;
                context.destroy();
                println!("method inactive");
            }
            _ => {}
        }
    }

    event_created_child!(AppState, zwp_input_method_v1::ZwpInputMethodV1, [
        EVT_ACTIVATE_OPCODE => (zwp_input_method_context_v1::ZwpInputMethodContextV1, ()),
    ]);
}

impl Dispatch<zwp_input_method_context_v1::ZwpInputMethodContextV1, ()> for AppState {
    fn event(
        state: &mut Self,
        _context: &zwp_input_method_context_v1::ZwpInputMethodContextV1,
        event: zwp_input_method_context_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        println!("current content event is {:#?}", event);
        match event {
            zwp_input_method_context_v1::Event::SurroundingText {
                text,
                cursor,
                anchor,
            } => {
                println!("{} {} {}", text, cursor, anchor);
            }
            zwp_input_method_context_v1::Event::CommitState { serial } => {
                state.input_serial = serial
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        state.process(event);
    }
}
