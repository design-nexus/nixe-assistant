//! Newline-delimited JSON over a Unix domain socket.

mod protocol;
mod server;

pub use protocol::{Command, Envelope};
pub use server::{
    bind, send_and_listen, send_command, send_until_done, stream_events, Ipc, Subscriber,
};
