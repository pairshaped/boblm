use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn background_bash_subcommand_decisions() {
    for (input, code) in [
        ("malformed", 0),
        (
            r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"run_in_background":false,"command":"work &"}}"#,
            0,
        ),
        (
            r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"run_in_background":true,"command":"work"}}"#,
            0,
        ),
        (
            r#"{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"run_in_background":true,"command":"work &"}}"#,
            2,
        ),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_boblm"))
            .arg("hook-background-bash")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(code));
        assert!(output.stdout.is_empty());
        if code == 2 {
            assert_eq!(
                String::from_utf8(output.stderr).unwrap(),
                "This command already runs as a background job; remove the inner &, nohup, setsid or disown and run the work directly, so the job ends when the work ends.\n"
            );
        } else {
            assert!(output.stderr.is_empty());
        }
    }
}
