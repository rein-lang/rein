use super::*;
use clap::CommandFactory;

#[test]
fn cli_reference_starts_with_notice_and_lists_every_command() {
    let doc = cli_reference();
    assert!(
        doc.starts_with(GENERATED_NOTICE),
        "missing generated notice"
    );
    for sub in Cli::command()
        .get_subcommands()
        .filter(|c| !c.is_hide_set())
    {
        let heading = format!("`rein {}`", sub.get_name());
        assert!(doc.contains(&heading), "reference lacks {heading}");
    }
}
