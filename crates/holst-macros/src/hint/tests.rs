use quote::quote;
use rstest::rstest;

use crate::{
    Tokens,
    hint::{FORWARD_FIELD, HINT_FORMS, NOT_ENUM, ONE_HINT, expand},
};

fn text(tokens: &Tokens) -> String {
    tokens.to_string()
}

fn expanded(item: Tokens) -> String {
    expand(item)
        .map(|tokens| tokens.to_string())
        .unwrap_or_default()
}

fn refusal(item: Tokens) -> String {
    expand(item)
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default()
}

#[test]
fn every_variant_shape_gets_an_arm() {
    let out = expanded(quote! {
        pub enum Sample {
            #[hint(LoginAgain)]
            Unit,
            #[hint(AskOwner)]
            Tuple(String, u8),
            #[hint(CheckId)]
            Named { id: String },
            Undecided,
            #[hint(forward)]
            Inner(Other),
            #[hint(forward)]
            Boxed { inner: Other },
        }
    });
    let wanted = [
        text(
            &quote!(pub const fn hint(&self) -> ::core::option::Option<crate::error::hint::Hint>),
        ),
        text(
            &quote!(Self::Unit => ::core::option::Option::Some(crate::error::hint::Hint::LoginAgain),),
        ),
        text(
            &quote!(Self::Tuple(..) => ::core::option::Option::Some(crate::error::hint::Hint::AskOwner),),
        ),
        text(
            &quote!(Self::Named { .. } => ::core::option::Option::Some(crate::error::hint::Hint::CheckId),),
        ),
        text(&quote!(Self::Undecided => ::core::option::Option::None,)),
        text(&quote!(Self::Inner(inner) => inner.hint(),)),
        text(&quote!(Self::Boxed { inner: inner } => inner.hint(),)),
    ];
    let missing: Vec<_> = wanted.iter().filter(|want| !out.contains(*want)).collect();
    assert!(missing.is_empty(), "{missing:?} in {out}");
}

#[test]
fn a_cfg_on_a_variant_goes_to_its_arm() {
    let out = expanded(quote! {
        pub enum Sample {
            #[cfg(not(target_os = "macos"))]
            #[hint(UseHome)]
            Linux(String),
        }
    });
    let wanted = text(&quote!(
        #[cfg(not(target_os = "macos"))]
        Self::Linux(..) => ::core::option::Option::Some(crate::error::hint::Hint::UseHome),
    ));
    assert!(out.contains(&wanted), "{out}");
}

#[test]
fn generics_are_kept() {
    let out = expanded(quote! {
        pub enum Sample<T: Clone> where T: Send {
            #[hint(CheckId)]
            One(T),
        }
    });
    let wanted = text(&quote!(impl<T: Clone> Sample<T> where T: Send));
    assert!(out.contains(&wanted), "{out}");
}

#[rstest]
#[case::a_struct(quote!(pub struct Sample { id: String }), NOT_ENUM)]
#[case::no_name(quote!(enum Sample { #[hint] One }), HINT_FORMS)]
#[case::two_names(quote!(enum Sample { #[hint(A, B)] One }), HINT_FORMS)]
#[case::a_value(quote!(enum Sample { #[hint = "A"] One }), HINT_FORMS)]
#[case::a_path(quote!(enum Sample { #[hint(forward = 1)] One(Other) }), HINT_FORMS)]
#[case::twice(quote!(enum Sample { #[hint(A)] #[hint(B)] One }), ONE_HINT)]
#[case::forward_unit(quote!(enum Sample { #[hint(forward)] One }), FORWARD_FIELD)]
#[case::forward_two(quote!(enum Sample { #[hint(forward)] One(A, B) }), FORWARD_FIELD)]
fn a_wrong_use_is_refused_with_its_reason(#[case] item: Tokens, #[case] reason: &str) {
    assert_eq!(refusal(item), reason);
}
