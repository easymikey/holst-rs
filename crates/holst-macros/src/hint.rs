//! The `Hint` derive: the advice under each variant of an error enum.

use proc_macro2::TokenStream as Tokens;
use quote::quote;
use syn::{Attribute, Fields, Ident, ItemEnum, Variant};

pub(crate) const NOT_ENUM: &str = "`Hint` takes an enum";
pub(crate) const HINT_FORMS: &str =
    "expected `#[hint(Name)]`, naming a `Hint`, or `#[hint(forward)]`";
pub(crate) const ONE_HINT: &str = "a variant takes one `#[hint(..)]`";
pub(crate) const FORWARD_FIELD: &str =
    "`#[hint(forward)]` needs a variant with exactly one field";
const ATTRIBUTE: &str = "hint";
const FORWARD: &str = "forward";

/// What a variant says about its advice.
enum Decision {
    None,
    Named(Ident),
    Forward,
}

impl Decision {
    fn of(variant: &Variant) -> syn::Result<Self> {
        let mut hints = variant
            .attrs
            .iter()
            .filter(|attribute| attribute.path().is_ident(ATTRIBUTE));
        let Some(attribute) = hints.next() else {
            return Ok(Self::None);
        };
        if let Some(second) = hints.next() {
            return Err(syn::Error::new_spanned(second, ONE_HINT));
        }
        let word: Ident = attribute
            .parse_args()
            .map_err(|_| syn::Error::new_spanned(attribute, HINT_FORMS))?;
        if word != FORWARD {
            return Ok(Self::Named(word));
        }
        if variant.fields.len() == 1 {
            Ok(Self::Forward)
        } else {
            Err(syn::Error::new_spanned(variant, FORWARD_FIELD))
        }
    }
}

fn conditions(variant: &Variant) -> impl Iterator<Item = &Attribute> {
    variant
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("cfg"))
}

fn arm(variant: &Variant) -> syn::Result<Tokens> {
    let name = &variant.ident;
    let cfgs = conditions(variant);
    let (pattern, result) = match (Decision::of(variant)?, &variant.fields) {
        (Decision::Forward, Fields::Named(fields)) => {
            let field = fields.named.iter().filter_map(|f| f.ident.as_ref());
            (
                quote!(Self::#name { #(#field: inner),* }),
                quote!(inner.hint()),
            )
        }
        (Decision::Forward, _) => (quote!(Self::#name(inner)), quote!(inner.hint())),
        (decision, fields) => {
            let pattern = match fields {
                Fields::Unit => quote!(Self::#name),
                Fields::Unnamed(_) => quote!(Self::#name(..)),
                Fields::Named(_) => quote!(Self::#name { .. }),
            };
            let result = match decision {
                Decision::Named(hint) => {
                    quote!(::core::option::Option::Some(crate::error::hint::Hint::#hint))
                }
                Decision::None | Decision::Forward => {
                    quote!(::core::option::Option::None)
                }
            };
            (pattern, result)
        }
    };
    Ok(quote!(#(#cfgs)* #pattern => #result,))
}

/// Expands `#[derive(Hint)]` on `item`.
pub(crate) fn expand(item: Tokens) -> syn::Result<Tokens> {
    let item: ItemEnum =
        syn::parse2(item).map_err(|error| syn::Error::new(error.span(), NOT_ENUM))?;
    let arms = item
        .variants
        .iter()
        .map(arm)
        .collect::<syn::Result<Vec<_>>>()?;
    let name = &item.ident;
    let (impl_generics, type_generics, where_clause) = item.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics #name #type_generics #where_clause {
            /// What to do next, decided for every variant.
            #[must_use]
            pub const fn hint(&self) -> ::core::option::Option<crate::error::hint::Hint> {
                match self {
                    #(#arms)*
                }
            }
        }
    })
}

#[cfg(test)] mod tests;
