use std::env;
use std::error::Error;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1).collect::<Vec<_>>();
    let mut socket = env::var_os("A26_SHELL_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/run/a26-shell/control.sock".into());

    if arguments.first().map(String::as_str) == Some("--socket") {
        if arguments.len() < 3 {
            return Err("usage: a26-shellctl [--socket PATH] COMMAND [ARGS...]".into());
        }
        socket = PathBuf::from(arguments.remove(1));
        arguments.remove(0);
    }
    if arguments.is_empty() {
        return Err("usage: a26-shellctl [--socket PATH] COMMAND [ARGS...]".into());
    }

    let mut stream = UnixStream::connect(socket)?;
    stream.write_all(arguments.join(" ").as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    print!("{response}");
    let parsed: serde_json::Value = serde_json::from_str(&response)?;
    if parsed.get("ok").and_then(|value| value.as_bool()) != Some(true) {
        std::process::exit(2);
    }
    Ok(())
}
