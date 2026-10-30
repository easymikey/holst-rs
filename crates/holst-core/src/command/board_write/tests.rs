#![expect(clippy::panic_in_result_fn, reason = "assert — the point of a test")]

use std::sync::{Arc, Mutex, PoisonError};

use futures_util::future;

use super::{WriteRequest, write, write_cached};
use crate::{
    command::board_load::tests::{
        body,
        cached_copy,
        cached_ids,
        linked_times,
        restored_server,
    },
    domain::{
        board::EditRight,
        change::{
            receipt::{BoardStatus, Delivery},
            value::BoardChange,
        },
        id_source::SequentialIds,
    },
    effect::{
        board_write::WriteInterpreter,
        board_write_plan::WrittenChange,
        cache::CachePort as _,
        clock::FixedClock,
        confirmation::{ALWAYS, ReleaseCheck},
    },
    error::{
        Error,
        Result,
        board_change::BoardChangeError,
        board_socket::{BoardSocketError, SocketErrorCode},
        http::HttpError,
    },
    protocol::{
        hud,
        hud::{DocumentResponse, SocketFrame, SocketMessage as HudMessage},
    },
    remote::{
        api::hud::UPLOAD_PATH,
        client::{Client, fake},
        socket::{
            fake::{
                BOARD,
                Script,
                Scripted,
                acking,
                answer,
                empty_board,
                note,
                on_board,
                on_board_reply,
                script,
                sent,
                signed_in,
            },
            recorder::WriteRecorder,
            write::{WriteOver, upload_error},
        },
    },
    storage::cache::memory::MemoryCache,
};

fn resetting(messages: impl IntoIterator<Item = Option<Vec<SocketFrame>>>) -> Scripted {
    let Scripted(script) = script(messages);
    let incoming = std::mem::take(
        &mut *script
            .incoming
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    Scripted(Arc::new(Script {
        incoming: Mutex::new(incoming),
        resets: true,
        ..Script::default()
    }))
}

type Cause = fn(&Error) -> bool;

#[tokio::test(start_paused = true)]
async fn a_failure_after_sending_is_an_unknown_outcome() -> Result<()> {
    let broken = Some(vec![SocketFrame {
        document: BOARD.to_owned(),
        message: HudMessage::Unknown {
            kind: 9,
            dropped: 0,
        },
    }]);
    let refused = Some(vec![answer(DocumentResponse::NotSubscribed {
        code: SocketErrorCode::InternalError,
    })]);
    let outcomes =
        future::join_all(unknown_outcome_cases(broken, refused).into_iter().map(
            |(socket, cause)| async move {
                let (_, client) = signed_in([])?;
                let client = client.with_ids(SequentialIds::new("u"));
                let failed =
                    written_by(&client, &WriteOver::sender(&socket), &note("hi")).await;
                Ok::<_, Error>((failed, cause))
            },
        ))
        .await;
    outcomes.into_iter().try_for_each(|outcome| {
        let (failed, cause) = outcome?;
        assert_unconfirmed(failed, cause);
        Ok(())
    })
}

fn assert_unconfirmed<T: std::fmt::Debug>(failed: Result<T>, cause: Cause) {
    let update = crate::domain::change::build::update_id("u-6");
    assert!(
        matches!(
            &failed,
            Err(Error::BoardChange(BoardChangeError::NotAccepted { update: got, objects, cause: inner }))
                if *got == update && objects == &["u-2"] && cause(inner)
        ),
        "{failed:?}"
    );
    let err = failed.err().map(|e| (e.to_string(), e.hint()));
    assert!(
        err.as_ref()
            .is_some_and(|(text, hint)| text.contains("may or may not")
                && *hint == Some(crate::error::hint::Hint::ReadBeforeRetry)),
        "{err:?}"
    );
}

fn unknown_outcome_cases(
    broken: Option<Vec<SocketFrame>>,
    refused: Option<Vec<SocketFrame>>,
) -> [(Scripted, Cause); 5] {
    [
        (script([Some(empty_board(EditRight::Editable))]), |cause| {
            matches!(
                cause,
                Error::BoardChange(BoardChangeError::AcceptTimeout { seconds: 15 })
            )
        }),
        (
            script([Some(empty_board(EditRight::Editable)), None]),
            |cause| {
                matches!(
                    cause,
                    Error::BoardChange(BoardChangeError::ClosedBeforeAccept)
                )
            },
        ),
        (
            resetting([Some(empty_board(EditRight::Editable))]),
            |cause| {
                matches!(cause, Error::BoardSocket(BoardSocketError::Connection(_)))
            },
        ),
        (
            script([Some(empty_board(EditRight::Editable)), broken]),
            |cause| {
                matches!(
                    cause,
                    Error::BoardSocket(BoardSocketError::MalformedFrame { .. })
                )
            },
        ),
        (
            script([Some(empty_board(EditRight::Editable)), refused]),
            |cause| {
                matches!(
                    cause,
                    Error::BoardSocket(BoardSocketError::NotSubscribed { .. })
                )
            },
        ),
    ]
}

#[test]
fn a_failed_upload_is_unknown_only_when_it_may_have_landed() {
    let maybe = Error::Http(HttpError::NoResponse {
        method: "POST",
        path: UPLOAD_PATH.into(),
    });
    let refused = Error::Http(HttpError::Status {
        method: "POST",
        path: UPLOAD_PATH.into(),
        status: 413,
        detail: String::new(),
    });
    assert!(matches!(
        upload_error(3, maybe),
        Error::BoardChange(BoardChangeError::NotAccepted { update: 3, .. })
    ));
    assert!(matches!(
        upload_error(3, refused),
        Error::Http(HttpError::Status { status: 413, .. })
    ));
}

async fn written_by<I: WriteInterpreter>(
    client: &Client,
    interpreter: &I,
    op: &BoardChange,
) -> Result<WrittenChange> {
    let account = crate::domain::account::Account::of(
        &crate::remote::api::user::current(client).await?,
    )?;
    write(
        client,
        interpreter,
        WriteRequest {
            board: BOARD,
            change: op,
            send_if: ALWAYS,
            account_id: &account.id,
            kept: None,
        },
    )
    .await
    .map(|changed| changed.written)
}

#[tokio::test]
async fn a_dry_run_sends_nothing_but_the_request() -> Result<()> {
    let (_, client) = signed_in([])?;
    let socket = acking([Some(empty_board(EditRight::Editable))]);
    let written =
        written_by(&client, &WriteOver::reporter(&socket), &note("hi")).await?;
    assert_eq!(written.delivery, Delivery::Built);
    let id = written
        .change
        .objects
        .first()
        .map(|t| t.id.clone())
        .unwrap_or_default();
    let bytes = written.change.update.len();
    let outcome = written.report(BOARD, |object| format!("L/{object}"));
    assert_eq!(
        outcome.to_string(),
        format!(
            "would create sticker {id}  L/{id}\ndry run, nothing sent: update of {bytes} \
                 bytes built"
        )
    );
    let report = serde_json::to_value(&outcome)?;
    assert_eq!(report.pointer("/dry_run"), Some(&serde_json::json!(true)));
    assert_eq!(
        report.pointer("/objects/0/change"),
        Some(&serde_json::json!("created"))
    );
    assert_eq!(outcome.board_status, BoardStatus::Live);
    assert_eq!(sent(&socket), [hud::request_document(BOARD, "tok")]);
    Ok(())
}

#[tokio::test]
async fn a_held_back_change_is_built_but_not_sent() -> Result<()> {
    let (_, client) = signed_in([])?;
    let socket = acking([Some(empty_board(EditRight::Editable))]);
    let held: ReleaseCheck = |_| false;
    let changed = write(
        &client,
        &WriteOver::sender(&socket),
        WriteRequest {
            board: BOARD,
            change: &note("hi"),
            send_if: held,
            account_id: "me",
            kept: None,
        },
    )
    .await?;
    assert_eq!(changed.written.delivery, Delivery::Built);
    assert_eq!(changed.written.change.objects.len(), 1);
    assert_eq!(sent(&socket), [hud::request_document(BOARD, "tok")]);
    Ok(())
}

#[tokio::test]
async fn a_write_succeeds_only_on_its_own_ack() -> Result<()> {
    let (_, client) = signed_in([])?;
    let other = Some(vec![on_board(HudMessage::Ack(1))]);
    let socket = acking([Some(empty_board(EditRight::Editable)), other]);
    let written = written_by(&client, &WriteOver::sender(&socket), &note("hi")).await?;
    let frames = sent(&socket);
    let update = frames
        .get(1)
        .map(|frame| hud::read_update_frame(frame))
        .transpose()?;
    assert_eq!(
        update,
        Some((
            BOARD.to_owned(),
            written.update_id,
            hud::OutgoingUpdate::Inline(written.change.update.clone())
        ))
    );
    assert_eq!(written.delivery, Delivery::Accepted);
    Ok(())
}

#[tokio::test]
async fn frames_of_other_documents_do_not_break_the_wait_for_the_ack() -> Result<()> {
    let (_, client) = signed_in([])?;
    let other = Some(vec![
        SocketFrame {
            document: format!("{BOARD}$state"),
            message: HudMessage::Ack(1),
        },
        SocketFrame {
            document: format!("{BOARD}$comments"),
            message: HudMessage::Unknown {
                kind: 9,
                dropped: 0,
            },
        },
    ]);
    let socket = acking([Some(empty_board(EditRight::Editable)), other]);
    let written = written_by(&client, &WriteOver::sender(&socket), &note("hi")).await?;
    assert_eq!(written.delivery, Delivery::Accepted);
    Ok(())
}

#[tokio::test]
async fn a_write_into_the_trash_is_marked() -> Result<()> {
    let trashed = r#"{"id":"b","in_trash":"2026-10-23T10:00:00Z"}"#;
    let (http, client) = on_board_reply(trashed, [])?;
    let recorder = WriteRecorder::new(EditRight::Editable);
    let written = written_by(&client, &recorder, &note("hi")).await?;
    assert_eq!(written.placement, BoardStatus::InTrash);
    assert!(
        recorder
            .plans()
            .iter()
            .all(|plan| plan.placement == BoardStatus::InTrash)
    );
    assert_eq!(
        written.report(BOARD, |_| String::new()).board_status,
        BoardStatus::InTrash
    );
    let asked = http.requests().get(1).map(|r| r.url.path().to_owned());
    assert_eq!(asked, Some(format!("/gapi/channel/{BOARD}")));
    Ok(())
}

#[tokio::test]
async fn a_failed_board_reply_is_no_trash() -> Result<()> {
    let me = fake::reply(200, &[], r#"{"id":"me"}"#);
    let (_, client) = fake::on_fake([me], Some(fake::stored("tok", "s=1")))?;
    let recorder = WriteRecorder::new(EditRight::Editable);
    let written = written_by(&client, &recorder, &note("hi")).await?;
    assert_eq!(written.placement, BoardStatus::Live);
    Ok(())
}

#[tokio::test]
async fn a_read_only_board_is_not_written() -> Result<()> {
    let (_, client) = signed_in([])?;
    let recorder = WriteRecorder::new(EditRight::ReadOnly);
    assert!(matches!(
        written_by(&client, &recorder, &note("hi")).await,
        Err(Error::BoardChange(BoardChangeError::ReadOnly))
    ));
    assert_eq!(recorder.plans(), [], "no plan carried out");
    Ok(())
}

#[tokio::test]
async fn a_large_update_is_uploaded_and_linked() -> Result<()> {
    let (http, client) = signed_in([fake::reply(200, &[], "")])?;
    let socket = acking([Some(empty_board(EditRight::Editable))]);
    let long = "x".repeat(hud::MAX_INLINE + 10);
    let written =
        written_by(&client, &WriteOver::sender(&socket), &note(&long)).await?;
    let upload = http.requests().into_iter().nth(2);
    let query = format!("documentName={BOARD}&updateId={}", written.update_id);
    assert_eq!(
        upload.as_ref().map(|r| (r.url.path(), r.url.query())),
        Some(("/gapi/hud/upload", Some(query.as_str())))
    );
    let frames = sent(&socket);
    let update = frames
        .get(1)
        .map(|frame| hud::read_update_frame(frame))
        .transpose()?;
    let size = u64::try_from(written.change.update.len()).unwrap_or_default();
    let link = format!("{BOARD}/{}", written.update_id);
    assert_eq!(
        update.map(|(_, _, payload)| payload),
        Some(hud::OutgoingUpdate::Uploaded { size, link })
    );
    Ok(())
}

#[tokio::test]
async fn a_write_updates_the_cache_after_the_ack() -> Result<()> {
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let (_, client) = signed_in([])?;
    let socket = acking([Some(empty_board(EditRight::Editable))]);
    let written = write_cached(
        &client,
        &WriteOver::sender(&socket),
        WriteRequest {
            board: BOARD,
            change: &note("hi"),
            send_if: ALWAYS,
            account_id: "me",
            kept: None,
        },
        Some(&memory),
    )
    .await?;
    assert_eq!(written.change.written.delivery, Delivery::Accepted);
    let state = memory.load_document(BOARD).unwrap_or_default();
    let doc = crate::domain::board_file::document::decoded(&state)?;
    let texts: Vec<String> = crate::domain::board::objects(&doc)
        .into_iter()
        .map(|object| object.text)
        .collect();
    assert_eq!(texts, ["hi"]);
    Ok(())
}

#[tokio::test]
async fn a_dry_run_is_not_cached() -> Result<()> {
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    let (_, client) = signed_in([])?;
    let socket = acking([Some(empty_board(EditRight::Editable))]);
    write_cached(
        &client,
        &WriteOver::reporter(&socket),
        WriteRequest {
            board: BOARD,
            change: &note("hi"),
            send_if: ALWAYS,
            account_id: "me",
            kept: None,
        },
        Some(&memory),
    )
    .await?;
    assert_eq!(memory.meta(BOARD), None);
    Ok(())
}

#[tokio::test]
async fn a_write_from_a_copy_ahead_of_the_server_heals() -> Result<()> {
    let (copy, server_sv, server) = restored_server();
    let memory = MemoryCache::new(1 << 20, FixedClock { unix_millis: 1 });
    cached_copy(&memory, &copy)?;
    let (_, writing) = signed_in([body(server)])?;
    let written = write_cached(
        &writing,
        &WriteOver::sender(&acking(linked_times(&server_sv, 2))),
        WriteRequest {
            board: BOARD,
            change: &note("hi"),
            send_if: ALWAYS,
            account_id: "me",
            kept: None,
        },
        Some(&memory),
    )
    .await?;
    assert_eq!(written.change.written.delivery, Delivery::Accepted);
    let kept = cached_ids(&memory)?;
    assert!(kept.contains(&"restored".to_owned()), "{kept:?}");
    assert!(!kept.contains(&"mine".to_owned()), "a write kept the copy");
    Ok(())
}
