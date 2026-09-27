//! The agent's system message.
//!
//! This string is resent with *every* model call, and a single user request can make a
//! dozen calls, so its size is multiplied before it ever reaches a provider. It was
//! previously ~6.6 KB of prose that stated most of its directives twice (once under
//! "Operating Principles", again under "Capabilities"), which made the system message a
//! large fixed tax on every turn. The directives are all still here; only the repetition
//! and the padding are gone. `identity_message_stays_within_its_budget` guards the size.

/// Current UTC date and time as a human-readable anchor for the model.
///
/// The system message previously carried no date at all, which pushed models
/// into guessing the year during time-sensitive research (release dates,
/// changelogs, "is this repo abandoned?"). Stdlib-only: the workspace has no
/// chrono/time dependency and this does not justify adding one.
pub(crate) fn current_utc_datetime_line() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0);
    format_utc_datetime(now)
}

/// Format a unix timestamp (seconds) as `YYYY-MM-DD HH:MM (Weekday) UTC`.
fn format_utc_datetime(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let secs_of_day = unix_secs.rem_euclid(86_400);
    // Civil-date conversion (Howard Hinnant's algorithm): days since the Unix
    // epoch to proleptic Gregorian year/month/day, no external crates.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let march_year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 {
        march_year + 1
    } else {
        march_year
    };
    // 1970-01-01 was a Thursday, so day 0 maps to index 0 here.
    const WEEKDAYS: [&str; 7] = [
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
    ];
    let weekday = WEEKDAYS[days.rem_euclid(7) as usize];
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} ({weekday}) UTC",
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60
    )
}

/// Build the system message.
///
/// Every bullet here is a distinct directive that was earned from a real failure. When
/// adding one, prefer tightening the wording of an existing bullet over appending.
pub(crate) fn system_message(agent_name: &str, installed_skill_ids: &[String]) -> String {
    let mut message = format!(
        "You are {agent_name}, an autonomous terminal coding agent with read and write \
access to a workspace.\n\
Installed skills are your capabilities, not your identity.\n\n\
Current date and time: {}. Treat this as the authoritative present moment when reasoning \
about deadlines, release dates, or research recency; never infer the current year from \
training data.\n\n\
HOW YOU WORK\n\
- Act, don't describe. When the user asks you to create, build, fix, or refactor, write \
the real file with `file.write` or `file.replace` in the same turn. Do not reply with code \
blocks for the user to copy, and do not state an intent without emitting the tool call.\n\
- Never fake tool output. Do not print JSON blobs or \"Tool X succeeded\" text as though a \
tool had run; call the tool.\n\
- Run it yourself. Execute commands, tests, and dev servers with the shell tools instead \
of telling the user to open a terminal. After starting a dev server, confirm it is up and \
report the localhost URL.\n\
- Research before guessing. If a task touches an unfamiliar library, API, platform, \
registry, or convention, call `web.fetch` first. Never invent endpoints or package names.\n\
- Inspect before editing code: use `project.scan` or `file.read` to see the existing \
layout and dependencies. Skip the scan for advisory, conceptual, or brainstorming \
questions and answer those directly.\n\
- Verify after writing. Call `test.run` (it auto-detects Cargo, npm, and pytest) and fix \
failures at the root cause rather than patching symptoms.\n\
- Deliver complete code. No placeholders, no `// TODO`, no `...` elisions.\n\
- `question.ask` is a last resort: use it only when a destructive or \
architecture-blocking choice truly cannot be resolved by you, or when the user explicitly \
asks for options. Never use it for ordinary questions, advice, or brainstorming, and never \
stack it after the user has already answered.\n\
- Keep preambles to one or two sentences, then call the tool.\n\
- Answer questions about your own identity, capabilities, and usage directly, without a \
tool. You may author a personalized skill with `skill.create` when a repeatable workflow \
appears.\n\
- Use each tool result to move forward. Never re-scan an empty workspace or repeat a \
question you have already asked.\n\
- On Windows, use PowerShell syntax: separate commands with `;` not `&&`, and use \
`$HOME` or `$env:USERPROFILE` rather than `~`.\n\
- Style: sharp, direct, technical. No filler such as \"Sure! I'd be happy to help\".\n\n\
TOOLS\n\
- Inspect: `project.scan`, `file.read`, `file.read_many`, `code.grep`, `code.glob`, \
`code.list`\n\
- Change: `file.write`, `file.replace`\n\
- Run: `shell.powershell.safe`, `shell.bash.safe`, `shell.zsh.safe`, `python.run`, \
`test.run`\n\
- Research: `web.fetch` (pass a `url` or a search `query`), `github.search`\n\
- Delegate and ask: `subagent.run` (read-only investigation), `question.ask`\n\
- Extend: `skill.create`\n\
- Read-only version control: `git.status`, `git.diff`\n\n\
Installed and currently available skill IDs:\n",
        current_utc_datetime_line()
    );

    if installed_skill_ids.is_empty() {
        message.push_str("- none\n");
    } else {
        for (index, skill_id) in installed_skill_ids.iter().enumerate() {
            message.push_str(&format!("{}. {skill_id}\n", index + 1));
        }
    }

    message
}

#[cfg(test)]
mod tests {
    use super::{format_utc_datetime, system_message};

    /// A realistic installed skill set, used to size the message under load.
    fn sample_skill_ids(count: usize) -> Vec<String> {
        (0..count)
            .map(|index| format!("workspace.skill-{index:02}.long-identifier"))
            .collect()
    }

    #[test]
    fn identity_message_names_axiom_and_all_available_skills() {
        let message = system_message(
            "Axiom Agent",
            &["file.read".to_string(), "git.status".to_string()],
        );

        assert!(message.contains("You are Axiom Agent"));
        assert!(message.contains("1. file.read"));
        assert!(message.contains("2. git.status"));
        assert!(
            message.contains("Current date and time: "),
            "system message must anchor the model to the real date"
        );
    }

    /// The system message rides along with every model call, so its size is multiplied
    /// by the number of calls in a turn. This guards against prose creeping back in.
    #[test]
    fn identity_message_stays_within_its_budget() {
        // Measured at 2,977 characters bare and 4,521 with 40 skills installed; the old
        // message exceeded this before its duplicated tool inventory was removed.
        let message = system_message("Axiom Agent", &sample_skill_ids(40));
        assert!(
            message.len() <= 5_000,
            "system message grew to {} characters; it is resent with every model call",
            message.len()
        );
    }

    /// Each directive came from a real failure. Losing one silently is a regression, so
    /// assert on the instruction itself rather than on any heading it used to live under.
    #[test]
    fn identity_message_keeps_every_operating_directive() {
        let message = system_message("Axiom Agent", &[]);
        for directive in [
            "write the real file",
            "Never fake tool output",
            "Run it yourself",
            "Research before guessing",
            "Inspect before editing code",
            "Verify after writing",
            "No placeholders",
            "question.ask` is a last resort",
            "without a tool",
            "Never re-scan an empty workspace",
            "$env:USERPROFILE",
            "No filler",
        ] {
            assert!(
                message.contains(directive),
                "system message lost the directive: {directive}"
            );
        }
    }

    #[test]
    fn utc_formatter_matches_known_timestamps() {
        assert_eq!(format_utc_datetime(0), "1970-01-01 00:00 (Thursday) UTC");
        // 1_000_000_000 was 2001-09-09 01:46:40 UTC, a Sunday.
        assert_eq!(
            format_utc_datetime(1_000_000_000),
            "2001-09-09 01:46 (Sunday) UTC"
        );
        // Leap-day: 2024-02-29 12:00:00 UTC was a Thursday.
        assert_eq!(
            format_utc_datetime(1_709_208_000),
            "2024-02-29 12:00 (Thursday) UTC"
        );
    }

    #[test]
    fn identity_message_handles_an_empty_skill_set() {
        let message = system_message("Axiom Agent", &[]);

        assert!(message.contains("- none"));
    }
}
