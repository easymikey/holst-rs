//! Comment threads of a board, the same on both surfaces.

use holst_core::fake::Right;
use holst_scenarios::{
    CommentCommand,
    FixtureObject,
    FixtureThread,
    Given,
    ScenarioError,
    Surface,
    ThreadAnchor,
    ThreadPin,
    comments_of,
    given,
    read_time_of,
    then,
};
use rstest::rstest;

const BOARD: &str = "Plan";
const READ: &str = "2026-10-26T10:00:00Z";
const AUTHOR: u32 = 7;

fn with_board() -> Given {
    given().saved_login().workspace("Team")
}

fn with_thread() -> Given {
    with_board().board(BOARD, "Team", None).thread(
        BOARD,
        FixtureThread::new("c1", AUTHOR, "Why?").at(10.0, 20.0),
    )
}

fn with_replied_thread() -> Given {
    with_board().board(BOARD, "Team", None).thread(
        BOARD,
        FixtureThread::new("c1", AUTHOR, "Why?")
            .at(10.0, 20.0)
            .reply(8, "Because"),
    )
}

#[rstest]
#[tokio::test]
async fn a_board_without_threads_lists_none(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_board().board(BOARD, "Team", None).start().await?;
    let observation = holst.on(surface).list_comments(BOARD).await?;
    then(&observation).shows("(empty)");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn threads_are_listed_with_replies_and_read_time(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_board()
        .board(BOARD, "Team", None)
        .thread(
            BOARD,
            FixtureThread::new("c1", AUTHOR, "Why?").at(10.0, 20.0),
        )
        .thread(
            BOARD,
            FixtureThread::new("c2", AUTHOR, "Done?")
                .at(30.0, 40.0)
                .reply(8, "Yes")
                .resolved()
                .read_at(READ),
        )
        .start()
        .await?;
    let observation = holst.on(surface).list_comments(BOARD).await?;
    then(&observation).shows_in_order(&[
        "Why?",
        "no replies",
        "unread",
        "resolved",
        "Done?",
        "1 reply",
        &format!("read {READ}"),
    ]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_thread_starts_at_a_point(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_board().board(BOARD, "Team", None).start().await?;
    let started = holst
        .on(surface)
        .add_comment(BOARD, "Why?", ThreadPin::At("10,20"))
        .await?;
    then(&started).succeeded();
    let threads = comments_of(&holst, BOARD);
    threads.count(1);
    threads.is_pinned_at(1, &ThreadAnchor::Point(10.0, 20.0));
    threads.has_messages(1, &["Why?"]);
    let listed = holst.on(surface).list_comments(BOARD).await?;
    then(&listed).shows("Why?");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_thread_starts_on_an_object(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_board()
        .board_with_objects(BOARD, "Team", vec![FixtureObject::Sticker("Idea".into())])
        .start()
        .await?;
    let started = holst
        .on(surface)
        .add_comment(BOARD, "Why?", ThreadPin::Object(1))
        .await?;
    then(&started).succeeded();
    let threads = comments_of(&holst, BOARD);
    threads.count(1);
    threads.is_pinned_at(1, &ThreadAnchor::Object(1));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_point_and_an_object_are_not_both_taken(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_board()
        .board_with_objects(BOARD, "Team", vec![FixtureObject::Sticker("Idea".into())])
        .start()
        .await?;
    let refused = holst
        .on(surface)
        .add_comment(BOARD, "Why?", ThreadPin::Both("10,20", 1))
        .await?;
    then(&refused).signalled_as(surface.usage());
    comments_of(&holst, BOARD).count(0);
    Ok(())
}

#[rstest]
#[case::reply(CommentCommand::Reply)]
#[case::resolve(CommentCommand::Resolve)]
#[case::remove(CommentCommand::Remove)]
#[case::read(CommentCommand::Read)]
#[tokio::test]
async fn a_command_on_an_unknown_thread_is_not_found(
    #[case] command: CommentCommand,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_thread().start().await?;
    let observation = holst
        .on(surface)
        .run_comment_command(BOARD, command, "@9")
        .await?;
    then(&observation).signalled_as(surface.not_found());
    let threads = comments_of(&holst, BOARD);
    threads.count(1);
    threads.has_messages(1, &["Why?"]);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_resolved_thread_can_be_reopened(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_thread().start().await?;
    let resolved = holst
        .on(surface)
        .resolve_comment(BOARD, "c1", false)
        .await?;
    then(&resolved).succeeded();
    comments_of(&holst, BOARD).is_resolved(1, true);
    let reopened = holst.on(surface).resolve_comment(BOARD, "c1", true).await?;
    then(&reopened).succeeded();
    comments_of(&holst, BOARD).is_resolved(1, false);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_thread_removal_needs_confirmation(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_replied_thread().start().await?;
    let observation = holst.on(surface).delete_comment(BOARD, "c1").await?;
    then(&observation)
        .failed_saying("deleting the thread c1")
        .signalled_as(surface.permission_denied());
    comments_of(&holst, BOARD).count(1);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_confirmed_removal_takes_the_thread_with_its_replies(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_replied_thread().start().await?;
    let removed = holst
        .on(surface)
        .confirming()
        .delete_comment(BOARD, "c1")
        .await?;
    then(&removed).succeeded();
    comments_of(&holst, BOARD).count(0);
    let listed = holst.on(surface).list_comments(BOARD).await?;
    then(&listed).shows("(empty)");
    Ok(())
}

#[rstest]
#[case::add(CommentCommand::Add)]
#[case::reply(CommentCommand::Reply)]
#[case::resolve(CommentCommand::Resolve)]
#[case::remove(CommentCommand::Remove)]
#[case::read(CommentCommand::Read)]
#[tokio::test]
async fn a_dry_run_of_a_comment_write_changes_nothing(
    #[case] command: CommentCommand,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_thread().start().await?;
    let observation = holst
        .on(surface)
        .dry_running()
        .run_comment_command(BOARD, command, "c1")
        .await?;
    then(&observation).shows("would");
    let threads = comments_of(&holst, BOARD);
    threads.count(1);
    threads.has_messages(1, &["Why?"]);
    threads.is_resolved(1, false);
    assert_eq!(read_time_of(&holst, BOARD, "c1"), None);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_thread_is_marked_read(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_thread().start().await?;
    let marked = holst
        .on(surface)
        .mark_comment_read(BOARD, "c1", None)
        .await?;
    then(&marked).succeeded();
    assert!(read_time_of(&holst, BOARD, "c1").is_some());
    let listed = holst.on(surface).list_comments(BOARD).await?;
    then(&listed).does_not_show("unread");
    let at = "2026-10-27T09:30:00Z";
    let timed = holst
        .on(surface)
        .mark_comment_read(BOARD, "c1", Some(at))
        .await?;
    then(&timed).succeeded();
    assert_eq!(read_time_of(&holst, BOARD, "c1").as_deref(), Some(at));
    let relisted = holst.on(surface).list_comments(BOARD).await?;
    then(&relisted).shows(&format!("read {at}"));
    Ok(())
}

#[rstest]
#[tokio::test]
async fn a_viewer_cannot_comment_a_commentator_can(
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let viewer = with_board()
        .board_as(BOARD, "Team", Right::Viewer)
        .start()
        .await?;
    let refused = viewer
        .on(surface)
        .add_comment(BOARD, "Why?", ThreadPin::Free)
        .await?;
    then(&refused).signalled_as(surface.permission_denied());
    comments_of(&viewer, BOARD).count(0);
    let commentator = with_board()
        .board_as(BOARD, "Team", Right::Commentator)
        .start()
        .await?;
    let started = commentator
        .on(surface)
        .add_comment(BOARD, "Why?", ThreadPin::Free)
        .await?;
    then(&started).succeeded();
    comments_of(&commentator, BOARD).count(1);
    Ok(())
}

#[rstest]
#[case::handle("@1")]
#[case::id("c1")]
#[tokio::test]
async fn a_comment_is_found_by_handle_or_id(
    #[case] thread: &str,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = with_thread().start().await?;
    let replied = holst
        .on(surface)
        .reply_in_comment(BOARD, thread, "Yes")
        .await?;
    then(&replied).succeeded();
    comments_of(&holst, BOARD).has_messages(1, &["Why?", "Yes"]);
    Ok(())
}

#[rstest]
#[case::ls(CommentCommand::Ls)]
#[case::add(CommentCommand::Add)]
#[case::reply(CommentCommand::Reply)]
#[case::resolve(CommentCommand::Resolve)]
#[case::remove(CommentCommand::Remove)]
#[case::read(CommentCommand::Read)]
#[tokio::test]
async fn comment_commands_need_a_login(
    #[case] command: CommentCommand,
    #[values(Surface::Cli, Surface::Mcp)] surface: Surface,
) -> Result<(), ScenarioError> {
    let holst = given()
        .workspace("Team")
        .board(BOARD, "Team", None)
        .start()
        .await?;
    let observation = holst
        .on(surface)
        .run_comment_command(BOARD, command, "@1")
        .await?;
    then(&observation)
        .failed_with_hint("log in again")
        .signalled_as(surface.permission_denied());
    Ok(())
}
