//! Asking the page service how many pages a file has, against the fake.

use holst_core::{
    command::login::connect,
    domain::{response::PagesInfo, session::Session},
    fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN, Fake, Fixture},
    remote::{api::files::page_count, client::Client},
    system::{clock::SystemClock, id::RandomIds},
};
use secrecy::SecretString;

async fn started(seed: Fixture) -> holst_core::Result<(Fake, Client)> {
    let fake = Fake::start(seed).await?;
    let session = Session::with_cookies(
        SecretString::from(ACCEPTED_TOKEN),
        ACCEPTED_COOKIES.parse().unwrap_or_default(),
    );
    let client = connect(
        Some(session),
        fake.base_url().parse()?,
        SystemClock,
        RandomIds,
    )?;
    Ok((fake, client))
}

#[tokio::test]
async fn the_reply_of_the_page_service_is_the_pages_info() -> holst_core::Result<()> {
    let (fake, client) =
        started(Fixture::default().page_count("a1.pdf", 3, true)).await?;
    assert_eq!(
        page_count(&client, "a1.pdf").await?,
        PagesInfo {
            count: 3,
            valid: true
        }
    );
    assert_eq!(
        fake.requests(),
        vec!["GET /pdfpage/file/a1.pdf/page_count".to_owned()]
    );
    Ok(())
}

#[tokio::test]
async fn a_file_the_service_cannot_convert_says_why() -> holst_core::Result<()> {
    let seed = Fixture::default().unconvertible_file(
        "a1.docx",
        "office_conversion_unavailable",
        "the converter is down",
    );
    let (_fake, client) = started(seed).await?;
    let refused = page_count(&client, "a1.docx").await.err();
    assert_eq!(
        refused.as_ref().and_then(holst_core::Error::status),
        Some(400)
    );
    let words = refused.map(|err| err.to_string()).unwrap_or_default();
    assert!(
        words.ends_with("office_conversion_unavailable: the converter is down"),
        "{words}"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_file_is_a_not_found() -> holst_core::Result<()> {
    let (_fake, client) = started(Fixture::default()).await?;
    let refused = page_count(&client, "gone.pdf").await.err();
    assert_eq!(refused.and_then(|err| err.status()), Some(404));
    Ok(())
}
