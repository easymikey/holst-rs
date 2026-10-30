//! `board edit` exits like every other change: the refusal is the exit code.

use holst_core::fake::{Fake, Fixture, Right};

use crate::support::{TestResult, holst_in, home, stderr};

fn viewer_fake() -> std::io::Result<Fake> {
    Fake::start_blocking(Fixture::default().workspace("easy").board_as(
        "Plans",
        "easy",
        Right::Viewer,
    ))
}

#[test]
fn a_board_the_user_may_not_change_is_refused_with_its_exit_code() -> TestResult {
    let fake = viewer_fake()?;
    let base = fake.base_url();
    let dir = home(None)?;
    let login = holst_in(base, &dir, &["auth", "login"], "mock@example.com\n123456\n")?;
    assert_eq!(login.status.code(), Some(0), "{}", stderr(&login));
    let edit = holst_in(base, &dir, &["board", "edit", "Plans", "--note", "hi"], "")?;
    let text = stderr(&edit);
    assert_eq!(edit.status.code(), Some(77), "{text}");
    assert!(text.starts_with("error: "), "{text}");
    Ok(())
}

#[test]
fn a_board_the_user_may_change_is_edited() -> TestResult {
    let fake = crate::fake::start()?;
    let base = fake.base_url();
    let dir = home(None)?;
    let login = holst_in(base, &dir, &["auth", "login"], "mock@example.com\n123456\n")?;
    assert_eq!(login.status.code(), Some(0), "{}", stderr(&login));
    let edit = holst_in(base, &dir, &["board", "edit", "Plans", "--note", "hi"], "")?;
    assert_eq!(edit.status.code(), Some(0), "{}", stderr(&edit));
    Ok(())
}
