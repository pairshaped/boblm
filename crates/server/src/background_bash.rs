use std::io::Read;

const MESSAGE: &str = "This command already runs as a background job; remove the inner &, nohup, setsid or disown and run the work directly, so the job ends when the work ends.";

pub fn run() -> i32 {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_ok() && blocks(&input) {
        eprintln!("{MESSAGE}");
        2
    } else {
        0
    }
}

fn blocks(input: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(input) else {
        return false;
    };
    value["hook_event_name"] == "PreToolUse"
        && value["tool_name"] == "Bash"
        && value["tool_input"]["run_in_background"] == true
        && value["tool_input"]["command"]
            .as_str()
            .is_some_and(detaches)
}

// Recognize shell words and control operators without executing or expanding input.
// Command-position tracking keeps ordinary arguments such as "echo nohup" valid.
fn detaches(command: &str) -> bool {
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;
    let mut command_position = true;
    let mut redirect_target = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c.is_whitespace() {
            if c == '\n' {
                command_position = true;
            }
            i += 1;
            continue;
        }
        if c == '&' {
            if chars.get(i + 1) == Some(&'&') {
                command_position = true;
                i += 2;
            } else if chars.get(i + 1) == Some(&'>') {
                i += 2;
                if chars.get(i) == Some(&'>') {
                    i += 1;
                }
                redirect_target = true;
            } else {
                return true;
            }
            continue;
        }
        if matches!(c, ';' | '|' | '(' | ')' | '{' | '}') {
            command_position = true;
            i += 1;
            if c == '|' && chars.get(i) == Some(&'&') {
                i += 1;
            }
            continue;
        }
        if matches!(c, '>' | '<') {
            i += 1;
            while i < chars.len() && matches!(chars[i], '>' | '<' | '&' | '|') {
                i += 1;
            }
            redirect_target = true;
            continue;
        }
        let start = i;
        let mut word = String::new();
        while i < chars.len()
            && !chars[i].is_whitespace()
            && !matches!(chars[i], '&' | ';' | '|' | '(' | ')' | '<' | '>')
        {
            match chars[i] {
                '\\' => {
                    i += 1;
                    if let Some(c) = chars.get(i) {
                        if *c != '\n' {
                            word.push(*c);
                        }
                        i += 1;
                    }
                }
                quote @ ('\'' | '"') => {
                    i += 1;
                    while i < chars.len() && chars[i] != quote {
                        if quote == '"' && chars[i] == '\\' {
                            i += 1;
                        }
                        if let Some(c) = chars.get(i) {
                            word.push(*c);
                            i += 1;
                        }
                    }
                    if i < chars.len() {
                        i += 1;
                    }
                }
                c => {
                    word.push(c);
                    i += 1;
                }
            }
        }
        if redirect_target {
            redirect_target = false;
        } else if !(word.chars().all(|c| c.is_ascii_digit())
            && matches!(chars.get(i), Some('>' | '<')))
            && command_position
        {
            let name = word.rsplit('/').next().unwrap_or(&word);
            if matches!(name, "nohup" | "setsid" | "disown") {
                return true;
            }
            // Assignments and shell command prefixes preserve command position.
            command_position = word.contains('=')
                || matches!(
                    word.as_str(),
                    "!" | "command"
                        | "exec"
                        | "env"
                        | "then"
                        | "do"
                        | "else"
                        | "if"
                        | "while"
                        | "until"
                );
        }
        if i == start {
            i += 1;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_bash_parser_table() {
        for (command, expected) in [
            ("work &", true),
            (
                "( bin/land > log 2>&1; echo done >> log ) > /dev/null 2>&1 & sleep 2; echo started",
                true,
            ),
            ("work && echo done", false),
            ("work |& cat", false),
            ("work 2>&1", false),
            ("work &> log", false),
            ("work &>> log", false),
            ("work >& log", false),
            ("echo 'a & b'", false),
            ("echo \"nohup setsid disown &\"", false),
            ("echo nohup setsid disown", false),
            ("echo a\\&b", false),
            ("# nohup work &\necho ok", false),
            ("nohup work", true),
            ("setsid work", true),
            ("work; disown", true),
            ("A=b /usr/bin/nohup work", true),
            ("command nohup work", true),
            ("echo ok | setsid work", true),
            ("2>log nohup work", true),
            ("echo x > nohup", false),
        ] {
            assert_eq!(detaches(command), expected, "{command}");
        }
    }

    #[test]
    fn background_bash_decisions_fail_open() {
        for input in [
            "",
            "{",
            "{}",
            "null",
            r#"{"tool_input":{"run_in_background":true,"command":"work &"}}"#,
        ] {
            assert!(!blocks(input));
        }
        for background in [false, true] {
            for command in ["work && echo done", "work &", "nohup work"] {
                let input = serde_json::json!({
                    "hook_event_name": "PreToolUse", "tool_name": "Bash",
                    "tool_input": {"command": command, "run_in_background": background},
                    "unknown": true
                });
                assert_eq!(
                    blocks(&input.to_string()),
                    background && command != "work && echo done"
                );
            }
        }
    }
}
