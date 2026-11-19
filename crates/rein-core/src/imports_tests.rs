use super::*;
use crate::ast::{Awaited, Callee, Param, ReturnValue, Stmt, StructRef};
use crate::parser::parse;

fn error_of(src: &str) -> String {
    let (_, diags) = parse(src);
    assert_eq!(diags.len(), 1, "{src}: {diags:?}");
    diags[0].message.clone()
}

fn function_with(imports: &str, body: &str) -> String {
    format!("{imports}\n/** Entry. */\nexport default function main() {{\n  {body}\n}}\n")
}

#[test]
fn named_import_records_the_declared_name_and_path() {
    let (f, diags) =
        parse("import { fix } from \"./ci/fix\";\n/** d */\nexport default function main() { return fix; }");
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(f.imports.len(), 1);
    let i = &f.imports[0];
    assert_eq!(i.name.value, "fix");
    assert_eq!(i.local.value, "fix");
    assert_eq!(i.path.value, "./ci/fix");
    assert!(!i.default && !i.aliased);
    assert!(i.used);
}

#[test]
fn as_gives_a_local_name() {
    let (f, diags) = parse(&function_with(
        "import { fix as ciFix } from \"./ci/fix\";",
        "return ciFix;",
    ));
    assert!(diags.is_empty(), "{diags:?}");
    let i = &f.imports[0];
    assert_eq!((i.name.value.as_str(), i.local.value.as_str()), ("fix", "ciFix"));
    assert!(i.aliased);
}

#[test]
fn default_import_resolves_under_its_local_name() {
    let (f, diags) = parse(&function_with("import fix from \"./ci/fix\";", "return fix;"));
    assert!(diags.is_empty(), "{diags:?}");
    let i = &f.imports[0];
    assert!(i.default);
    assert_eq!((i.name.value.as_str(), i.local.value.as_str()), ("fix", "fix"));
}

#[test]
fn struct_path_refers_to_the_struct_file_without_its_suffix() {
    let (f, _) = parse(&function_with(
        "import { Message } from \"./message.struct\";",
        "return `x`;",
    ));
    let i = &f.imports[0];
    assert!(i.is_struct());
    assert_eq!(i.reference(), "./message");
}

#[test]
fn optional_semicolon_and_program_path_parse() {
    let (f, diags) = parse(&function_with(
        "import { prHandling } from \"pr-handling\"",
        "return prHandling(pr=`x`);",
    ));
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(f.imports[0].path.value, "pr-handling");
}

#[test]
fn return_of_an_import_is_the_reference_it_names() {
    let (f, diags) = parse(&function_with("import { fix } from \"./ci/fix\";", "return fix;"));
    assert!(diags.is_empty(), "{diags:?}");
    let Stmt::Return(Spanned {
        value: ReturnValue::Ref(path),
        ..
    }) = &f.body[0]
    else {
        panic!("{:?}", f.body)
    };
    assert_eq!(path.value, "./ci/fix");
}

#[test]
fn call_of_an_import_is_a_call_of_its_reference() {
    let (f, diags) = parse(&function_with("import { tpl } from \"./tpl\";", "return tpl(a=`x`);"));
    assert!(diags.is_empty(), "{diags:?}");
    let Stmt::Return(Spanned {
        value: ReturnValue::Call(call),
        ..
    }) = &f.body[0]
    else {
        panic!("{:?}", f.body)
    };
    assert!(
        matches!(&call.callee, Callee::Ref(p) if p.value == "./tpl"),
        "{:?}",
        call.callee
    );
}

#[test]
fn await_of_an_import_is_an_await_of_its_reference() {
    let src = "import { merge } from \"./merge\";\n/** d */\nexport default async function main() {\n  /** Merged? */\n  const ok: boolean = await merge(pr=`x`);\n  /** Again? */\n  const again: boolean = await merge;\n  return `done`;\n}\n";
    let (f, diags) = parse(src);
    assert!(diags.is_empty(), "{diags:?}");
    let Stmt::Request {
        awaited: Awaited::Call(ReturnValue::Call(call)),
        ..
    } = &f.body[0]
    else {
        panic!("{:?}", f.body)
    };
    assert!(matches!(&call.callee, Callee::Ref(p) if p.value == "./merge"));
    let Stmt::Request {
        awaited: Awaited::Call(ReturnValue::Ref(p)),
        ..
    } = &f.body[1]
    else {
        panic!("{:?}", f.body)
    };
    assert_eq!(p.value, "./merge");
}

#[test]
fn struct_import_is_the_struct_type_of_a_parameter_and_an_answer() {
    let src = "import { Message } from \"./message.struct\";\n/** d */\nexport default function tell({ opening }: Message) {\n  return `${opening}`;\n}\n";
    let (f, diags) = parse(src);
    assert!(diags.is_empty(), "{diags:?}");
    let Some(Param::Struct(p)) = f.params.first() else {
        panic!("{:?}", f.params)
    };
    assert_eq!(
        p.ty,
        StructRef::Ref(Spanned {
            value: "./message".into(),
            span: p.ty.name().span
        })
    );
}

#[test]
fn struct_import_as_answer_type_is_a_struct_reference() {
    let src = "import { Message } from \"./message.struct\";\n/** d */\nexport default async function main() {\n  /** Next? */\n  const next: Message = await request(`x`);\n  return `done`;\n}\n";
    let (f, diags) = parse(src);
    assert!(diags.is_empty(), "{diags:?}");
    let answers = f.answer_struct_refs();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].1.name().value, "./message");
}

#[test]
fn imports_must_come_first() {
    let msg = error_of("/** d */\nimport { fix } from \"./ci/fix\";\nexport default function main() { return `x`; }");
    assert_eq!(msg, IMPORTS_FIRST_TEXT);
    let msg = error_of("/** d */\nexport default function main() { return `x`; }\nimport { fix } from \"./ci/fix\";");
    assert_eq!(msg, IMPORTS_FIRST_TEXT);
}

const IMPORTS_FIRST_TEXT: &str = "imports come first; move this `import` above the doc comment and the declarations";

#[test]
fn import_that_is_not_a_name_of_the_target_is_checked_later_not_here() {
    // The parser keeps the name as written. The project checks it against the target.
    let (f, diags) = parse(&function_with("import { fixCi } from \"./ci/fix\";", "return `x`;"));
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(f.imports[0].name.value, "fixCi");
}

#[test]
fn several_names_in_braces_are_an_error() {
    let msg = error_of(&function_with(
        "import { fix, merge } from \"./ci/fix\";",
        "return `x`;",
    ));
    assert!(msg.starts_with("the braces hold one name"), "{msg}");
}

#[test]
fn import_type_is_an_error_that_names_the_form() {
    for src in [
        "import type { Message } from \"./message.struct\";",
        "import type Message from \"./message.struct\";",
    ] {
        let msg = error_of(&function_with(src, "return `x`;"));
        assert_eq!(
            msg,
            "`import type` is not supported; write `import { name } from \"path\";`"
        );
    }
}

#[test]
fn namespace_import_is_an_error_that_names_the_form() {
    let msg = error_of(&function_with("import * as ci from \"./ci/fix\";", "return `x`;"));
    assert_eq!(
        msg,
        "`import *` is not supported; write `import { name } from \"path\";`"
    );
}

#[test]
fn side_effect_import_is_an_error_that_names_the_form() {
    let msg = error_of(&function_with("import \"./ci/fix\";", "return `x`;"));
    assert_eq!(
        msg,
        "a side-effect import is not supported; write `import { name } from \"path\";`"
    );
}

#[test]
fn export_from_is_an_error_that_names_the_form() {
    for src in ["export { fix } from \"./ci/fix\";", "export * from \"./ci/fix\";"] {
        let msg = error_of(&format!(
            "{src}\n/** d */\nexport default function main() {{ return `x`; }}"
        ));
        assert_eq!(
            msg,
            "`export ... from` is not supported; write `import { name } from \"path\";`"
        );
    }
}

#[test]
fn path_with_rein_extension_is_an_error() {
    let msg = error_of(&function_with("import { fix } from \"./ci/fix.rein\";", "return `x`;"));
    assert_eq!(msg, "an import has no `.rein` extension; write `\"./ci/fix\"`");
}

#[test]
fn path_must_be_relative_or_a_program_name() {
    let msg = error_of(&function_with("import { fix } from \"ci-fix!\";", "return `x`;"));
    assert!(msg.starts_with("a program is imported by its name"), "{msg}");
    let msg = error_of(&function_with("import { fix } from \"ci/fix\";", "return `x`;"));
    assert!(msg.contains("names a function inside a program"), "{msg}");
    let msg = error_of(&function_with(
        "import { main } from \"pr-handling/main\";",
        "return `x`;",
    ));
    assert!(msg.contains("names a function inside a program"), "{msg}");
    assert!(msg.contains("call-programs-by-entry"), "{msg}");
}

#[test]
fn bad_relative_path_is_an_error() {
    let msg = error_of(&function_with("import { fix } from \"./ci/fi-x\";", "return `x`;"));
    assert!(msg.starts_with("a path is `\"./path\"`"), "{msg}");
}

#[test]
fn std_name_cannot_be_imported_or_used_as_a_local_name() {
    let msg = error_of(&function_with(
        "import { delegate } from \"./delegate\";",
        "return `x`;",
    ));
    assert_eq!(msg, "`delegate` is a std name; std names need no import");
    let msg = error_of(&function_with(
        "import { fix as askUser } from \"./ci/fix\";",
        "return `x`;",
    ));
    assert_eq!(msg, "`askUser` is a std name; std names need no import");
}

#[test]
fn two_imports_with_one_local_name_are_an_error() {
    let src = function_with(
        "import { fix } from \"./ci/fix\";\nimport { fix } from \"./other/fix\";",
        "return `x`;",
    );
    assert_eq!(error_of(&src), "`fix` is imported twice; each local name is one import");
}

#[test]
fn local_name_cannot_match_a_declaration_of_the_same_file() {
    let src = "import { Message } from \"./message.struct\";\n/** d */\ninterface Message {\n  /** x */\n  a: string;\n}\nexport default function main({ a }: Message) { return `${a}`; }\n";
    assert_eq!(error_of(src), "`Message` is already declared in this file");
}

#[test]
fn struct_import_cannot_be_called_or_returned() {
    let msg = error_of(&function_with(
        "import { Message } from \"./message.struct\";",
        "return Message;",
    ));
    assert!(msg.contains("`Message` is a struct, not a function"), "{msg}");
    let msg = error_of(&function_with(
        "import { Message } from \"./message.struct\";",
        "return Message(a=`x`);",
    ));
    assert!(msg.contains("`Message` is a struct, not a function"), "{msg}");
}

#[test]
fn function_import_cannot_be_a_parameter_type() {
    let src = "import { fix } from \"./ci/fix\";\n/** d */\nexport default function main({ a }: fix) { return `x`; }\n";
    let msg = error_of(src);
    assert!(msg.contains("`fix` is a function, not a struct"), "{msg}");
}

#[test]
fn function_import_cannot_be_an_answer_type() {
    let src = "import { fix } from \"./ci/fix\";\n/** d */\nexport default async function main() {\n  /** Next? */\n  const next: fix = await request(`x`);\n  return `x`;\n}\n";
    let msg = error_of(src);
    assert!(msg.contains("`fix` is a function, not a struct"), "{msg}");
}

#[test]
fn unused_import_parses_and_is_marked_unused() {
    let (f, diags) = parse(&function_with("import { fix } from \"./ci/fix\";", "return `x`;"));
    assert!(diags.is_empty(), "{diags:?}");
    assert!(!f.imports[0].used);
}

#[test]
fn named_import_keeps_the_name_when_the_path_has_no_dots() {
    let (f, diags) = parse(&function_with("import { ci_fix } from \"../../x\";", "return `x`;"));
    assert!(diags.is_empty(), "{diags:?}");
    assert_eq!(f.imports[0].path.value, "../../x");
}

#[test]
fn name_with_a_dot_is_an_error() {
    let msg = error_of(&function_with("import { pick.pr } from \"./ci/fix\";", "return `x`;"));
    assert!(msg.contains("a name is one word"), "{msg}");
}

#[test]
fn path_error_names_the_valid_forms() {
    assert_eq!(path_error("./ci/fix"), None);
    assert_eq!(path_error("../escalate"), None);
    assert_eq!(path_error("./message.struct"), None);
    assert_eq!(path_error("pr-handling"), None);
    assert!(path_error("./a.struct.rein").is_some());
    assert!(path_error("./a.js").is_some());
    assert!(path_error("./").is_some());
    assert!(path_error("./a/").is_some());
    assert!(path_error("-x").is_some());
}
