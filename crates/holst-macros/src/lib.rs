//! Endpoint attributes and the `CommandArgs` derive for `holst-core`.

mod command;

use std::collections::BTreeSet;

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::{format_ident, quote};
use syn::{
    Attribute,
    Expr,
    FnArg,
    Ident,
    LitStr,
    Meta,
    Pat,
    Signature,
    Token,
    Visibility,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

/// A `GET` endpoint; the path is relative to `/gapi/`.
///
/// ```text
/// /// The workspace's boards and folders.
/// #[get("workspace/{workspace_id}/tree")]
/// pub async fn tree(client: &Client, workspace_id: &str) -> Result<Value>;
///
/// #[get("recent/channels/page", query(pageSize = size, cursor? = cursor))]
/// pub async fn page(client: &Client, size: u32, cursor: Option<&str>) -> Result<Value>;
///
/// #[get("audit_events", query = query.iter())]
/// pub async fn audit(client: &Client, query: &[(&str, String)]) -> Result<Value>;
///
/// #[get("workspace/{workspace_id}/tree", response = text)]
/// pub async fn tree_text(client: &Client, workspace_id: &str) -> Result<String>;
///
/// #[post("logout", anonymous)]
/// pub async fn bye(client: &Client) -> Result<Value>;
/// ```
#[proc_macro_attribute]
pub fn get(attribute: TokenStream, item: TokenStream) -> TokenStream {
    endpoint("Get", attribute, item)
}

#[proc_macro_attribute]
pub fn post(attribute: TokenStream, item: TokenStream) -> TokenStream {
    endpoint("Post", attribute, item)
}

#[proc_macro_attribute]
pub fn patch(attribute: TokenStream, item: TokenStream) -> TokenStream {
    endpoint("Patch", attribute, item)
}

#[proc_macro_attribute]
pub fn delete(attribute: TokenStream, item: TokenStream) -> TokenStream {
    endpoint("Delete", attribute, item)
}

/// The command-table entry of an arguments struct: `const COMMAND: Command`.
///
/// Removes about 300 argument declarations over about 50 commands that would
/// otherwise be written twice, as a serde field and as a table record; a table
/// cannot see a field's name or type, so the pair would drift without the
/// compiler noticing.
///
/// ```text
/// #[derive(Debug, Deserialize, CommandArgs)]
/// #[holst_command(group = Auth, verb = "status", risk = Read,
///     about = "Show the saved login", help = "…", example = "holst auth status")]
/// pub struct StatusArgs {
///     #[argument(value_name = "parts", help = "Also show these parts")]
///     pub with: Vec<StatusPart>,
/// }
/// ```
#[proc_macro_derive(CommandArgs, attributes(holst_command, argument))]
pub fn command_args(item: TokenStream) -> TokenStream {
    command::expand(item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Arguments several commands share: `const ARGUMENTS: &[Argument]`, which
/// `#[argument(flatten)]` on a field of a `CommandArgs` struct joins in.
#[proc_macro_derive(ArgumentGroup, attributes(argument))]
pub fn argument_group(item: TokenStream) -> TokenStream {
    command::expand_group(item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn endpoint(method: &str, attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand(method, attribute.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

const EXPECTED: &str = "expected `query(key = value, key? = option, ..)`, `query = <pairs>`, \
                        one `body = <json>` or `payload = <HttpBody>`, `response = text`, \
                        `response = typed`, `anonymous` or `request_only`";

struct EndpointQueryParameter {
    name: Ident,
    value: Expr,
}

enum EndpointQueryPart {
    Parameter(EndpointQueryParameter),
    OptionalParameter(EndpointQueryParameter),
    Parameters(Expr),
}

impl Parse for EndpointQueryPart {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let key: Ident = input.parse()?;
        let optional = input.parse::<Option<Token![?]>>()?;
        input.parse::<Token![=]>()?;
        let value: Expr = input.parse()?;
        let parameter = EndpointQueryParameter { name: key, value };
        Ok(match optional {
            Some(_) => Self::OptionalParameter(parameter),
            None => Self::Parameter(parameter),
        })
    }
}

enum EndpointBody {
    Json(Expr),
    Payload(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointResponse {
    Json,
    Text,
    Typed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointAuthorization {
    Session,
    Anonymous,
}

struct EndpointAttribute {
    path: LitStr,
    query: Vec<EndpointQueryPart>,
    body: Option<EndpointBody>,
    response: EndpointResponse,
    authorization: EndpointAuthorization,
    request_only: bool,
}

impl Parse for EndpointAttribute {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path = input.parse()?;
        let rest = input
            .parse::<Option<Token![,]>>()?
            .map(|_| Punctuated::<Meta, Token![,]>::parse_terminated(input))
            .transpose()?
            .unwrap_or_default();
        let empty = Self {
            path,
            query: Vec::new(),
            body: None,
            response: EndpointResponse::Json,
            authorization: EndpointAuthorization::Session,
            request_only: false,
        };
        rest.iter().try_fold(empty, Self::with)
    }
}

impl EndpointAttribute {
    fn with(self, meta: &Meta) -> syn::Result<Self> {
        let wrong = || syn::Error::new_spanned(meta, EXPECTED);
        match meta {
            Meta::List(list) if list.path.is_ident("query") => {
                let items = list.parse_args_with(
                    Punctuated::<EndpointQueryPart, Token![,]>::parse_terminated,
                )?;
                Ok(self.queried(items))
            }
            Meta::NameValue(pair) if pair.path.is_ident("query") => {
                Ok(self.queried([EndpointQueryPart::Parameters(pair.value.clone())]))
            }
            Meta::NameValue(pair) if pair.path.is_ident("body") => self
                .bodied(EndpointBody::Json(pair.value.clone()))
                .ok_or_else(wrong),
            Meta::NameValue(pair) if pair.path.is_ident("payload") => self
                .bodied(EndpointBody::Payload(pair.value.clone()))
                .ok_or_else(wrong),
            Meta::NameValue(pair)
                if pair.path.is_ident("response") && is_word(&pair.value, "text") =>
            {
                Ok(Self {
                    response: EndpointResponse::Text,
                    ..self
                })
            }
            Meta::NameValue(pair)
                if pair.path.is_ident("response") && is_word(&pair.value, "typed") =>
            {
                Ok(Self {
                    response: EndpointResponse::Typed,
                    ..self
                })
            }
            Meta::Path(path) if path.is_ident("anonymous") => Ok(Self {
                authorization: EndpointAuthorization::Anonymous,
                ..self
            }),
            Meta::Path(path) if path.is_ident("request_only") => Ok(Self {
                request_only: true,
                ..self
            }),
            Meta::List(_) | Meta::NameValue(_) | Meta::Path(_) => Err(wrong()),
        }
    }

    fn queried(self, more: impl IntoIterator<Item = EndpointQueryPart>) -> Self {
        Self {
            query: self.query.into_iter().chain(more).collect(),
            ..self
        }
    }

    fn bodied(self, body: EndpointBody) -> Option<Self> {
        self.body.is_none().then_some(Self {
            body: Some(body),
            ..self
        })
    }
}

fn is_word(expr: &Expr, word: &str) -> bool {
    matches!(expr, Expr::Path(path) if path.path.is_ident(word))
}

struct EndpointDeclaration {
    attrs: Vec<Attribute>,
    vis: Visibility,
    sig: Signature,
}

impl Parse for EndpointDeclaration {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let vis = input.parse()?;
        let sig = input.parse()?;
        input.parse::<Token![;]>().map(|_| Self { attrs, vis, sig })
    }
}

fn expand(method: &str, attribute: Tokens, item: Tokens) -> syn::Result<Tokens> {
    let endpoint: EndpointAttribute = syn::parse2(attribute)?;
    let EndpointDeclaration { attrs, vis, sig } = syn::parse2(item)?;
    let params = parameter_names(&sig)?;
    let (client, rest) = params.split_first().ok_or_else(|| {
        syn::Error::new_spanned(&sig.ident, "the first parameter must be the client")
    })?;
    let segments = placeholders(&endpoint.path.value())
        .into_iter()
        .map(|name| segment(&endpoint.path, &sig.ident, rest, &name))
        .collect::<syn::Result<BTreeSet<_>>>()?;
    let request = format_ident!("{}_request", sig.ident);
    let inputs = sig.inputs.iter().skip(1);
    let generics = &sig.generics;
    let where_clause = &sig.generics.where_clause;
    let build = build(method, &endpoint, &segments);
    let send = match endpoint.response {
        EndpointResponse::Json => format_ident!("send_json"),
        EndpointResponse::Text => format_ident!("send_text"),
        EndpointResponse::Typed => format_ident!("send_typed"),
    };
    let request_doc = if endpoint.request_only {
        "The request, as data: the endpoint has no sender of its own.".to_owned()
    } else {
        format!("The request [`{}`] sends, as data.", sig.ident)
    };
    let (request_attrs, sender) = if endpoint.request_only {
        (quote!(#(#attrs)*), quote!())
    } else {
        let sender = quote! {
            #(#attrs)*
            #[doc = "# Errors"]
            #[doc = "[`crate::remote::client::Client::send_json`] errors; [`crate::error::input::InputError::NotPathSegment`] for a bad id."]
            #vis #sig {
                #client.#send(#request(#(#rest),*)?).await
            }
        };
        (quote!(), sender)
    };
    Ok(quote! {
        #request_attrs
        #[doc = #request_doc]
        #[doc = "# Errors"]
        #[doc = "[`crate::error::input::InputError::NotPathSegment`] for a bad id; [`crate::error::http::HttpError::Json`] for a body."]
        #vis fn #request #generics(#(#inputs),*) -> crate::error::Result<crate::effect::request::ApiRequest>
        #where_clause
        {
            #build
        }

        #sender
    })
}

fn build(
    method: &str,
    endpoint: &EndpointAttribute,
    segments: &BTreeSet<Ident>,
) -> Tokens {
    let local = |name: &str| Ident::new(name, Span::mixed_site());
    let (query, body, path) = (local("query"), local("body"), local("path"));
    let parts = endpoint.query.iter().map(query_part);
    let payload = match &endpoint.body {
        None => quote!(::std::option::Option::None),
        Some(EndpointBody::Json(json)) => quote! {
            ::std::option::Option::Some(crate::effect::http::HttpBody::json(&(#json))?)
        },
        Some(EndpointBody::Payload(payload)) => {
            quote!(::std::option::Option::Some(#payload))
        }
    };
    let format = LitStr::new(
        &format!("/gapi/{}", endpoint.path.value()),
        endpoint.path.span(),
    );
    let method = format_ident!("{method}");
    let authorization = match endpoint.authorization {
        EndpointAuthorization::Session => quote!(Session),
        EndpointAuthorization::Anonymous => quote!(Anonymous),
    };
    quote! {
        let #query: ::std::vec::Vec<crate::effect::request::QueryParameter> =
            ::std::iter::empty()#(.chain(#parts))*
                .map(|(name, value)| crate::effect::request::QueryParameter { name, value })
                .collect();
        let #body: ::std::option::Option<crate::effect::http::HttpBody> = #payload;
        let #path = {
            #(let #segments = crate::domain::url::encode_segment(#segments)?;)*
            ::std::format!(#format)
        };
        ::std::result::Result::Ok(crate::effect::request::ApiRequest {
            method: crate::effect::http::HttpMethod::#method,
            path: #path,
            query: #query,
            body: #body,
            authorization: crate::effect::request::Authorization::#authorization,
        })
    }
}

fn query_part(part: &EndpointQueryPart) -> Tokens {
    let text = quote!(::std::string::ToString::to_string);
    match part {
        EndpointQueryPart::Parameter(EndpointQueryParameter { name: key, value }) => {
            let key = key.to_string();
            quote!([(::std::string::String::from(#key), #text(&(#value)))])
        }
        EndpointQueryPart::OptionalParameter(EndpointQueryParameter {
            name: key,
            value,
        }) => {
            let key = key.to_string();
            quote! {
                ::std::option::Option::map(#value, |value| {
                    (::std::string::String::from(#key), #text(&value))
                })
            }
        }
        EndpointQueryPart::Parameters(pairs) => quote! {
            ::std::iter::IntoIterator::into_iter(#pairs)
                .map(|(key, value)| (#text(&key), #text(&value)))
        },
    }
}

fn parameter_names(sig: &Signature) -> syn::Result<Vec<&Ident>> {
    sig.inputs
        .iter()
        .map(|arg| match arg {
            FnArg::Typed(typed) => match &*typed.pat {
                Pat::Ident(pat) => Ok(&pat.ident),
                Pat::Const(_)
                | Pat::Lit(_)
                | Pat::Macro(_)
                | Pat::Or(_)
                | Pat::Paren(_)
                | Pat::Path(_)
                | Pat::Range(_)
                | Pat::Reference(_)
                | Pat::Rest(_)
                | Pat::Slice(_)
                | Pat::Struct(_)
                | Pat::Tuple(_)
                | Pat::TupleStruct(_)
                | Pat::Type(_)
                | Pat::Verbatim(_)
                | Pat::Wild(_)
                | _ => Err(syn::Error::new_spanned(
                    &typed.pat,
                    "a parameter is one name",
                )),
            },
            FnArg::Receiver(receiver) => Err(syn::Error::new_spanned(
                receiver,
                "an endpoint is a free function",
            )),
        })
        .collect()
}

fn placeholders(template: &str) -> Vec<String> {
    template
        .split('{')
        .skip(1)
        .filter_map(|rest| rest.split_once('}'))
        .map(|(name, _)| name.to_owned())
        .collect()
}

fn segment(
    path: &LitStr,
    function: &Ident,
    parameters: &[&Ident],
    name: &str,
) -> syn::Result<Ident> {
    parameters
        .iter()
        .find(|param| **param == name)
        .map(|param| (*param).clone())
        .ok_or_else(|| {
            syn::Error::new(
                path.span(),
                format!("`{{{name}}}` in the path is not a parameter of `{function}`"),
            )
        })
}

#[cfg(test)]
mod tests {
    use quote::quote;
    use rstest::rstest;

    use crate::{EXPECTED, Tokens, expand, placeholders};

    fn expanded(method: &str, attribute: Tokens, item: Tokens) -> syn::Result<String> {
        expand(method, attribute, item).map(|tokens| tokens.to_string())
    }

    fn get(attribute: Tokens, item: Tokens) -> String {
        expanded("Get", attribute, item).unwrap_or_default()
    }

    fn text(tokens: &Tokens) -> String {
        tokens.to_string()
    }

    #[test]
    fn request_only_emits_the_builder_and_no_sender() {
        let out = get(
            quote!("channel/{channel_id}/leave", request_only),
            quote!(
                pub async fn leave(client: &Client, channel_id: &str) -> Result<Value>;
            ),
        );
        assert!(out.contains(&text(&quote!(pub fn leave_request(channel_id: &str)))));
        assert!(!out.contains("async fn leave"), "{out}");
    }

    #[test]
    fn ids_are_encoded_and_the_prefix_is_added() {
        let out = get(
            quote!("workspace/{workspace_id}/tree"),
            quote!(
                pub async fn tree(client: &Client, workspace_id: &str)
                -> Result<Value>;
            ),
        );
        let wanted = [
            text(&quote!(pub fn tree_request(workspace_id: &str))),
            text(
                &quote!(let workspace_id = crate::domain::url::encode_segment(workspace_id)?;),
            ),
            "\"/gapi/workspace/{workspace_id}/tree\"".to_owned(),
            text(&quote!(method: crate::effect::http::HttpMethod::Get)),
            text(
                &quote!(authorization: crate::effect::request::Authorization::Session),
            ),
            text(&quote!(client.send_json(tree_request(workspace_id)?).await)),
        ];
        let missing: Vec<_> = wanted.iter().filter(|w| !out.contains(*w)).collect();
        assert!(missing.is_empty(), "{missing:?} in {out}");
    }

    #[test]
    fn query_pairs_body_and_payload_are_built() {
        let out = expanded(
            "Post",
            quote!(
                "user/list",
                query(user_ids = user_ids.join(",")),
                body = json!({})
            ),
            quote!(
                pub async fn list(c: &Client, user_ids: &[String]) -> Result<Value>;
            ),
        )
        .unwrap_or_default();
        let pair = text(&quote! {
            [(::std::string::String::from("user_ids"),
              ::std::string::ToString::to_string(&(user_ids.join(","))))]
        });
        let body = text(&quote! {
            ::std::option::Option::Some(crate::effect::http::HttpBody::json(&(json!({})))?)
        });
        assert!(out.contains(&pair), "{out}");
        assert!(out.contains(&body), "{out}");
        assert!(out.contains("Method :: Post"), "{out}");
        let raw = get(
            quote!("upload_file", payload = bytes),
            quote!(
                pub async fn up(c: &Client, bytes: HttpBody) -> Result<Value>;
            ),
        );
        let payload = text(&quote!(::std::option::Option::Some(bytes)));
        assert!(raw.contains(&payload), "{raw}");
    }

    #[test]
    fn optional_query_is_sent_only_when_some() {
        let out = get(
            quote!("recent/page", query(pageSize = size, cursor? = cursor)),
            quote!(
                pub async fn page(
                    c: &Client,
                    size: u32,
                    cursor: Option<&str>,
                ) -> Result<Value>;
            ),
        );
        let optional = text(&quote! {
            ::std::option::Option::map(cursor, |value| {
                (::std::string::String::from("cursor"),
                 ::std::string::ToString::to_string(&value))
            })
        });
        let pair = text(&quote!(::std::string::String::from("pageSize")));
        assert!(out.contains(&optional), "{out}");
        assert!(
            out.find(&pair) < out.find(&optional),
            "the declared order: {out}"
        );
    }

    #[test]
    fn query_expression_is_passed_through() {
        let out = get(
            quote!("audit_events", query = query.iter()),
            quote!(
                pub async fn audit(
                    c: &Client,
                    query: &[(&str, String)],
                ) -> Result<Value>;
            ),
        );
        let pairs = text(&quote! {
            ::std::iter::IntoIterator::into_iter(query.iter())
                .map(|(key, value)| (
                    ::std::string::ToString::to_string(&key),
                    ::std::string::ToString::to_string(&value)
                ))
        });
        assert!(out.contains(&pairs), "{out}");
    }

    #[test]
    fn text_response_and_anonymous_change_the_call() {
        let out = expanded(
            "Post",
            quote!("login/with_email", anonymous, response = text),
            quote!(
                pub async fn login(c: &Client) -> Result<String>;
            ),
        )
        .unwrap_or_default();
        assert!(
            out.contains(&text(
                &quote!(authorization: crate::effect::request::Authorization::Anonymous)
            )),
            "{out}"
        );
        assert!(
            out.contains(&text(&quote!(c.send_text(login_request()?).await))),
            "{out}"
        );
    }

    #[rstest]
    #[case::typo_in_path(
        quote!("workspace/{workspace}"),
        quote!(pub async fn get(client: &Client, workspace_id: &str) -> Result<Value>;),
        "`{workspace}` in the path is not a parameter of `get`"
    )]
    #[case::client_is_not_a_segment(
        quote!("x/{client}"),
        quote!(pub async fn get(client: &Client) -> Result<Value>;),
        "`{client}` in the path is not a parameter of `get`"
    )]
    #[case::no_client(
        quote!("x"),
        quote!(pub async fn get() -> Result<Value>;),
        "the first parameter must be the client"
    )]
    #[case::pattern_parameter(
        quote!("x"),
        quote!(pub async fn get(client: &Client, (a, b): (u8, u8)) -> Result<Value>;),
        "a parameter is one name"
    )]
    #[case::unknown_argument(
        quote!("x", headers(a = b)),
        quote!(pub async fn get(client: &Client) -> Result<Value>;),
        EXPECTED
    )]
    #[case::two_bodies(
        quote!("x", body = 1, body = 2),
        quote!(pub async fn get(client: &Client) -> Result<Value>;),
        EXPECTED
    )]
    #[case::body_and_payload(
        quote!("x", body = 1, payload = p),
        quote!(pub async fn get(client: &Client) -> Result<Value>;),
        EXPECTED
    )]
    #[case::unknown_response(
        quote!("x", response = bytes),
        quote!(pub async fn get(client: &Client) -> Result<Value>;),
        EXPECTED
    )]
    #[case::path_query_key(
        quote!("x", query(a::b = 1)),
        quote!(pub async fn get(client: &Client) -> Result<Value>;),
        "expected `=`"
    )]
    fn mistakes_are_compile_errors(
        #[case] attribute: Tokens,
        #[case] item: Tokens,
        #[case] message: &str,
    ) {
        let error = expanded("Get", attribute, item)
            .err()
            .map(|err| err.to_string());
        assert_eq!(error.as_deref(), Some(message));
    }

    #[test]
    fn placeholders_are_read_in_order() {
        assert_eq!(placeholders("a/{x}/b/{y}"), ["x", "y"]);
        assert!(placeholders("plain/path").is_empty());
    }
}
