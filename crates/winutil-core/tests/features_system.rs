use winutil_core::features::system::dism_result;

#[test]
fn dism_success_and_reboot_required_are_both_success() {
    assert_eq!(dism_result(0, ""), Ok(()));
    assert_eq!(dism_result(3010, "Reboot required"), Ok(()));
}

#[test]
fn other_dism_codes_report_the_last_output_line() {
    assert_eq!(
        dism_result(740, "Error: 740\nElevated rights are required.\n\n"),
        Err("DISM exit code 740: Elevated rights are required.".to_owned())
    );
}
