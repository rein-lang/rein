use super::*;
use crate::parser::parse;

#[test]
fn snake_case_splits_words_and_keeps_acronyms_together() {
    assert_eq!(snake_case("isWorkInProgress"), "is_work_in_progress");
    assert_eq!(snake_case("ownPr"), "own_pr");
    assert_eq!(snake_case("prURL"), "pr_url");
    assert_eq!(snake_case("URLPath"), "url_path");
    assert_eq!(snake_case("step2Done"), "step2_done");
    assert_eq!(snake_case("already_snake"), "already_snake");
}

#[test]
fn camel_case_parameter_struct_fields_and_answers_warn() {
    let src = "interface Pick {\n  /** Done? */\n  isDone: boolean;\n  /** Next. */\n  next_step: string;\n}\n\n\
               interface Input {\n  /** The link. */\n  prLink: string;\n  /** Draft? */\n  is_draft: boolean;\n}\n\n\
               /** Try names. */\nexport default async function main({ prLink, is_draft }: Input) {\n  if (is_draft) {\n    \
               return `wait`;\n  }\n  /** What? */\n  const pickResult: Pick = await request(`look at ${prLink}`);\n  \
               if (pickResult.isDone) {\n    return `done`;\n  }\n  return `not done`;\n}\n";
    let (f, diags) = parse(src);
    assert!(diags.is_empty(), "{diags:?}");
    let msgs: Vec<String> = camel_case_names(&f).into_iter().map(|d| d.message).collect();
    // Struct fields first, in file order, then the answers. `next_step` and `is_draft` are quiet.
    assert_eq!(
        msgs,
        [
            "`isDone` is camelCase; names are snake_case: write `is_done`",
            "`prLink` is camelCase; names are snake_case: write `pr_link`",
            "`pickResult` is camelCase; names are snake_case: write `pick_result`",
        ]
    );
}

#[test]
fn a_scalar_parameter_warns_and_snake_case_is_quiet() {
    let camel = "/** Ask. */\nexport default function ask(\n  /** Draft? */\n  isDraft: boolean,\n) {\n  \
                 if (isDraft) {\n    return `a`;\n  }\n  return `b`;\n}\n";
    let (f, _) = parse(camel);
    let msgs: Vec<String> = camel_case_names(&f).into_iter().map(|d| d.message).collect();
    assert_eq!(msgs, ["`isDraft` is camelCase; names are snake_case: write `is_draft`"]);

    let snake = camel.replace("isDraft", "is_draft");
    let (f, _) = parse(&snake);
    assert!(camel_case_names(&f).is_empty());
}
