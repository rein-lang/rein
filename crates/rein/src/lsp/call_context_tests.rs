use super::*;

#[test]
fn call_at_reads_the_open_call() {
    let text = "  return @./tpl(a=`x, (y`, b";
    let call = call_at(text, text.len()).unwrap();
    assert_eq!(call.callee, "./tpl");
    assert_eq!(call.index, 1);
    assert_eq!(call.given, ["a"]);
    assert_eq!(call.current, None);
    assert!(call.at_name);
}

#[test]
fn call_at_skips_closed_calls_and_conditions() {
    assert!(call_at("delegate(model=`a`) ", 20).is_none());
    assert!(call_at("  if (vip", 9).is_none());
    let text = "delegate(model=\"a";
    let call = call_at(text, text.len()).unwrap();
    assert_eq!(call.current.as_deref(), Some("model"));
    assert!(!call.at_name);
}

#[test]
fn call_at_reads_a_program_reference() {
    let text = "  return @pr-handling/main(pr=pr, ";
    let call = call_at(text, text.len()).unwrap();
    assert_eq!(call.callee, "pr-handling/main");
    assert_eq!(call.given, ["pr"]);
}
