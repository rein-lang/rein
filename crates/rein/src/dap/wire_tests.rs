use super::*;
use std::io::Cursor;

#[test]
fn round_trips_one_framed_message() {
    let mut buf = Vec::new();
    write(&mut buf, &json!({"type": "request", "command": "threads"})).unwrap();
    let mut input = Cursor::new(buf);
    let msg = read(&mut input).unwrap().unwrap();
    assert_eq!(command(&msg), "threads");
    assert!(read(&mut input).unwrap().is_none());
}

#[test]
fn missing_length_header_is_an_error() {
    let mut input = Cursor::new(b"X-Other: 1\r\n\r\n{}".to_vec());
    assert!(read(&mut input).is_err());
}

#[test]
fn response_echoes_request_seq_and_command() {
    let mut sender = Sender::default();
    let req = json!({"seq": 7, "type": "request", "command": "launch"});
    let res = sender.fail(&req, "no program");
    assert_eq!(res["request_seq"], 7);
    assert_eq!(res["command"], "launch");
    assert_eq!(res["success"], false);
    assert_eq!(res["message"], "no program");
}
