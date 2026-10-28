//! Templates, sharing, uploads and backups against the fake Holst server.

use holst_core::{
    command::{login::connect, stored_file},
    domain::{response::Uploaded, session::Session, template::Template},
    remote::{
        api::{
            self,
            files::{UploadBind, UploadOptions},
        },
        client::Client,
    },
    system::{clock::SystemClock, id::RandomIds},
};
use holst_fake::{ACCEPTED_COOKIES, ACCEPTED_TOKEN, Fake, Fixture};
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

const fn text_upload() -> UploadOptions<'static> {
    UploadOptions {
        key: None,
        freshness: None,
        content_type: "text/plain",
    }
}

/// A personal template of `name`; the server wants a workspace for every scope.
fn named(name: &str) -> Template {
    let fields = serde_json::json!({ "name": name, "workspace_id": "w" });
    serde_json::from_value(fields).unwrap_or_default()
}

fn names(list: &holst_core::domain::template::Templates) -> Vec<&str> {
    list.templates
        .iter()
        .flatten()
        .filter_map(|template| template.name.as_deref())
        .collect()
}

#[tokio::test]
async fn a_template_is_created_listed_and_deleted() -> holst_core::Result<()> {
    let (fake, client) = started(Fixture::default().workspace("Team")).await?;
    client
        .send_json(api::template::create_request(&named("Sprint"))?)
        .await?;
    let listed = api::template::personal(&client).await?;
    assert_eq!(names(&listed), ["Sprint"]);
    let id = listed
        .templates
        .iter()
        .flatten()
        .next()
        .and_then(|template| template.id.clone())
        .unwrap_or_default();
    assert_eq!(
        api::template::get(&client, &id).await?.name.as_deref(),
        Some("Sprint")
    );
    client
        .send_json(api::template::delete_request(&id)?)
        .await?;
    assert!(fake.personal_templates().is_empty());
    Ok(())
}

#[tokio::test]
async fn an_uploaded_file_is_downloaded_from_the_storage() -> holst_core::Result<()> {
    let (fake, client) = started(Fixture::default().workspace("Team")).await?;
    let uploaded: Uploaded = api::files::upload(
        &client,
        UploadBind::User("u1"),
        text_upload(),
        "notes.txt",
        b"hello".to_vec(),
    )
    .await?;
    assert_eq!(fake.uploads().len(), 1);
    let address = format!(
        "{}{}",
        client.base_url().storage(),
        uploaded.name.unwrap_or_default()
    );
    assert_eq!(
        stored_file::download(&client, &address, 100).await?,
        b"hello"
    );
    Ok(())
}
