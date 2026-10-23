use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

pub type Result<T> = std::result::Result<T, String>;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

pub fn home_dir() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn socket_path() -> PathBuf {
    env::var_os("HERDR_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join(".config/herdr/herdr.sock"))
}

fn send(method: &str, params: Value, timeout: Option<Duration>) -> Result<BufReader<UnixStream>> {
    let mut stream = UnixStream::connect(socket_path()).map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(timeout)
        .map_err(|error| error.to_string())?;
    let request = json!({ "id": method, "method": method, "params": params });
    writeln!(stream, "{request}").map_err(|error| error.to_string())?;
    Ok(BufReader::new(stream))
}

fn read_response(reader: &mut BufReader<UnixStream>) -> Result<Value> {
    let mut line = String::new();
    if reader
        .read_line(&mut line)
        .map_err(|error| error.to_string())?
        == 0
    {
        return Err("herdr closed the connection".into());
    }
    let mut response: Value = serde_json::from_str(&line).map_err(|error| error.to_string())?;
    if let Some(error) = response.get("error") {
        return Err(error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| error.to_string()));
    }
    Ok(response["result"].take())
}

pub fn rpc(method: &str, params: Value) -> Result<Value> {
    read_response(&mut send(method, params, Some(REQUEST_TIMEOUT))?)
}

pub fn subscribe(event_types: &[&str], mut on_event: impl FnMut()) -> Result<()> {
    let subscriptions: Vec<Value> = event_types
        .iter()
        .map(|kind| json!({ "type": kind }))
        .collect();
    let mut reader = send(
        "events.subscribe",
        json!({ "subscriptions": subscriptions }),
        None,
    )?;
    read_response(&mut reader)?;
    loop {
        read_response(&mut reader)?;
        on_event();
    }
}
