use rstest::rstest;

use crate::login::page::http::{Form, Refusal, Request, read_request};

async fn read(raw: &str) -> Result<Request, Refusal> {
    read_request(&mut raw.as_bytes()).await
}

#[tokio::test]
async fn a_get_is_read_without_its_query() {
    let request = read("GET /login/x?y=1 HTTP/1.1\r\nHost: 127.0.0.1:9\r\n\r\n").await;
    assert_eq!(
        request,
        Ok(Request {
            method: "GET".to_owned(),
            path: "/login/x".to_owned(),
            host: Some("127.0.0.1:9".to_owned()),
            content_type: None,
            body: Vec::new(),
        })
    );
}

#[tokio::test]
async fn a_post_body_is_as_long_as_the_header_says() {
    let raw = "POST /p HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\n\
               Content-Length: 5\r\n\r\na=b&cTRAILING";
    let request = read(raw).await;
    assert_eq!(
        request.map(|request| (request.content_type, request.body)),
        Ok((
            Some("application/x-www-form-urlencoded".to_owned()),
            b"a=b&c".to_vec()
        ))
    );
}

#[rstest]
#[case::chunked(
    "POST /p HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
    Refusal::Malformed
)]
#[case::two_hosts("GET /p HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n", Refusal::Malformed)]
#[case::two_lengths(
    "POST /p HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\na",
    Refusal::Malformed
)]
#[case::not_http("hello\r\n\r\n", Refusal::Malformed)]
#[case::header_without_colon("GET /p HTTP/1.1\r\nHost\r\n\r\n", Refusal::Malformed)]
#[case::length_not_a_number(
    "POST /p HTTP/1.1\r\nContent-Length: many\r\n\r\n",
    Refusal::Malformed
)]
#[case::body_over_the_cap(
    "POST /p HTTP/1.1\r\nContent-Length: 4097\r\n\r\n",
    Refusal::BodyTooLarge
)]
#[case::body_cut_short(
    "POST /p HTTP/1.1\r\nContent-Length: 9\r\n\r\nabc",
    Refusal::Closed
)]
#[case::nothing("", Refusal::Closed)]
#[tokio::test]
async fn a_request_that_cannot_be_read_is_refused(
    #[case] raw: &str,
    #[case] refusal: Refusal,
) {
    assert_eq!(read(raw).await, Err(refusal));
}

#[tokio::test]
async fn a_head_without_an_end_is_cut_off() {
    let raw = format!("GET /p HTTP/1.1\r\nX: {}", "a".repeat(9_000));
    assert_eq!(read(&raw).await, Err(Refusal::HeadTooLarge));
}

#[rstest]
#[case("email=a%40b.c&secret=1+2", "email", Some("a@b.c"))]
#[case("email=a%40b.c&secret=1+2", "secret", Some("1 2"))]
#[case("secret=100%", "secret", Some("100%"))]
#[case("secret=%zz%e2%82", "secret", Some("%zz\u{fffd}"))]
#[case("email=x", "secret", None)]
fn a_form_is_read_field_by_field(
    #[case] body: &str,
    #[case] name: &str,
    #[case] value: Option<&str>,
) {
    assert_eq!(Form::parse(body.as_bytes()).get(name), value);
}
