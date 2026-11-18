//! Ultimate Performance power plan: the `powercfg` steps behind the two power buttons.
//!
//! Mirrors `Invoke-WPFUltimatePerformance`. The process runner is passed in, so the flow is
//! tested without starting `powercfg`.

use std::sync::LazyLock;

use regex::Regex;

/// The scheme GUID `powercfg /duplicatescheme` expects for Ultimate Performance.
pub const ULTIMATE_PERFORMANCE_GUID: &str = "e9a42b02-d5df-448d-aa00-03f14749eb61";

static SCHEME_GUID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Fa-f0-9-]{36}").expect("valid pattern"));

/// The first 36-character hex-and-dash run in `output`: the new scheme GUID that
/// `powercfg /duplicatescheme` prints.
pub fn parse_scheme_guid(output: &str) -> Option<&str> {
    SCHEME_GUID.find(output).map(|m| m.as_str())
}

/// Runs a program and returns its exit code and output, or `None` if it could not start.
pub type Run<'a> = dyn FnMut(&str, &[&str]) -> Option<(i32, String)> + 'a;

/// Duplicates the Ultimate Performance scheme and makes it active.
pub fn enable(run: &mut Run<'_>) -> Result<(), String> {
    let (code, output) = run(
        "powercfg",
        &["/duplicatescheme", ULTIMATE_PERFORMANCE_GUID],
    )
    .ok_or("powercfg could not be started.")?;
    if code != 0 {
        return Err(format!(
            "powercfg could not duplicate the Ultimate Performance scheme (exit code {code})."
        ));
    }
    let guid = parse_scheme_guid(&output)
        .ok_or("powercfg did not report a scheme GUID to activate.")?;
    let (code, _) = run("powercfg", &["/setactive", guid]).ok_or("powercfg could not be started.")?;
    if code != 0 {
        return Err(format!(
            "powercfg could not activate the Ultimate Performance scheme (exit code {code})."
        ));
    }
    Ok(())
}

/// Restores the default power schemes, which removes the Ultimate Performance plan.
pub fn disable(run: &mut Run<'_>) -> Result<(), String> {
    let (code, _) =
        run("powercfg", &["/restoredefaultschemes"]).ok_or("powercfg could not be started.")?;
    if code != 0 {
        return Err(format!(
            "powercfg could not restore the default power schemes (exit code {code})."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUPLICATE_OUTPUT: &str =
        "Power Scheme GUID: 12345678-1234-4234-8234-123456789abc  (Ultimate Performance)";

    /// A runner that answers each call from `replies`, in order, and records what was asked.
    fn scripted(
        replies: Vec<Option<(i32, String)>>,
    ) -> (impl FnMut(&str, &[&str]) -> Option<(i32, String)>, std::rc::Rc<std::cell::RefCell<Vec<Vec<String>>>>) {
        let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = calls.clone();
        let mut replies = replies.into_iter();
        let run = move |program: &str, args: &[&str]| {
            let mut line = vec![program.to_owned()];
            line.extend(args.iter().map(|a| (*a).to_owned()));
            log.borrow_mut().push(line);
            replies.next().flatten()
        };
        (run, calls)
    }

    #[test]
    fn parses_the_guid_from_duplicate_output() {
        assert_eq!(
            parse_scheme_guid(DUPLICATE_OUTPUT),
            Some("12345678-1234-4234-8234-123456789abc")
        );
    }

    #[test]
    fn no_guid_in_output_yields_none() {
        assert_eq!(parse_scheme_guid("access denied"), None);
    }

    #[test]
    fn enable_duplicates_then_activates_the_reported_guid() {
        let (mut run, calls) = scripted(vec![
            Some((0, DUPLICATE_OUTPUT.to_owned())),
            Some((0, String::new())),
        ]);
        enable(&mut run).unwrap();
        let calls = calls.borrow();
        assert_eq!(
            calls[0],
            ["powercfg", "/duplicatescheme", ULTIMATE_PERFORMANCE_GUID]
        );
        assert_eq!(
            calls[1],
            ["powercfg", "/setactive", "12345678-1234-4234-8234-123456789abc"]
        );
    }

    #[test]
    fn enable_reports_duplicate_exit_code() {
        let (mut run, _) = scripted(vec![Some((1, String::new()))]);
        assert_eq!(
            enable(&mut run).unwrap_err(),
            "powercfg could not duplicate the Ultimate Performance scheme (exit code 1)."
        );
    }

    #[test]
    fn enable_reports_missing_guid() {
        let (mut run, _) = scripted(vec![Some((0, "nothing useful".into()))]);
        assert_eq!(
            enable(&mut run).unwrap_err(),
            "powercfg did not report a scheme GUID to activate."
        );
    }

    #[test]
    fn enable_reports_activation_exit_code() {
        let (mut run, _) = scripted(vec![
            Some((0, DUPLICATE_OUTPUT.to_owned())),
            Some((5, String::new())),
        ]);
        assert_eq!(
            enable(&mut run).unwrap_err(),
            "powercfg could not activate the Ultimate Performance scheme (exit code 5)."
        );
    }

    #[test]
    fn enable_reports_powercfg_not_started() {
        let (mut run, _) = scripted(vec![None]);
        assert_eq!(enable(&mut run).unwrap_err(), "powercfg could not be started.");
    }

    #[test]
    fn disable_restores_default_schemes() {
        let (mut run, calls) = scripted(vec![Some((0, String::new()))]);
        disable(&mut run).unwrap();
        assert_eq!(calls.borrow()[0], ["powercfg", "/restoredefaultschemes"]);
    }

    #[test]
    fn disable_reports_exit_code() {
        let (mut run, _) = scripted(vec![Some((2, String::new()))]);
        assert_eq!(
            disable(&mut run).unwrap_err(),
            "powercfg could not restore the default power schemes (exit code 2)."
        );
    }
}
