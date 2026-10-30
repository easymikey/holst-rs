use quote::quote;
use rstest::rstest;

use crate::{
    Tokens,
    command::{
        ARGUMENT_KEYS,
        COMMAND_KEYS,
        FLATTEN_ALONE,
        KINDS_NEED_SUBCOMMAND,
        NAMED_FIELDS,
        NO_NESTED_GROUP,
        ONE_SUBCOMMAND,
        expand,
        expand_group,
    },
};

fn text(tokens: &Tokens) -> String {
    tokens.to_string()
}

#[test]
fn two_fields_become_the_command_entry() {
    let out = expand(quote! {
        #[holst_command(group = Auth, verb = "status", risk = Read,
            about = "Show it", help = "Shows it.", example = "holst auth status")]
        pub struct StatusArgs {
            #[argument(value_name = "parts", help = "Add parts")]
            pub with: Vec<StatusPart>,
            #[argument(letter = 'w', help = "The workspace", tool_doc = "Where.")]
            pub r#workspace: Option<String>,
        }
    })
    .map(|tokens| tokens.to_string())
    .unwrap_or_default();
    let wanted = [
        text(&quote!(impl crate::command::table::CommandArgs for StatusArgs)),
        text(&quote!(group: crate::command::table::CommandGroup::Auth)),
        text(&quote!(verb: ::core::option::Option::Some("status"))),
        text(&quote!(examples: &["holst auth status"])),
        text(&quote!(block: crate::command::table::HelpBlock::Commands)),
        text(&quote!(
            surfaces: &[crate::error::hint::Surface::Cli, crate::error::hint::Surface::Mcp]
        )),
        text(&quote!(name: "with", letter: ::core::option::Option::None)),
        text(
            &quote!(kind: <Vec<StatusPart> as crate::command::table::ArgumentValue>::KIND),
        ),
        text(&quote!(value_name: "parts")),
        text(&quote!(name: "workspace", letter: ::core::option::Option::Some('w'))),
        text(&quote!(value_name: "workspace")),
        text(&quote!(tool_doc: ::core::option::Option::Some("Where."))),
    ];
    let missing: Vec<_> = wanted.iter().filter(|want| !out.contains(*want)).collect();
    assert!(missing.is_empty(), "{missing:?} in {out}");
}

#[test]
fn a_flattened_group_is_joined_in_field_order() {
    let out = expand(quote! {
        #[holst_command(group = Board, verb = "show", risk = Read, about = "a", help = "b")]
        pub struct ShowArgs {
            #[argument(help = "The board")]
            pub board: String,
            #[serde(flatten)]
            #[argument(flatten)]
            pub selection: Selection,
        }
    })
    .map(|tokens| tokens.to_string())
    .unwrap_or_default();
    let wanted = [
        text(&quote!(const PARTS: &[&[crate::command::table::Argument]])),
        text(&quote!(
            <Selection as crate::command::table::ArgumentGroup>::ARGUMENTS
        )),
        text(&quote!(crate::command::table::joined(PARTS))),
    ];
    let missing: Vec<_> = wanted.iter().filter(|want| !out.contains(*want)).collect();
    assert!(missing.is_empty(), "{missing:?} in {out}");
}

#[test]
fn a_group_is_its_arguments() {
    let out = expand_group(quote! {
        pub struct Selection {
            #[argument(value_name = "x,y,w,h", help = "An area")]
            pub rectangle: Option<String>,
        }
    })
    .map(|tokens| tokens.to_string())
    .unwrap_or_default();
    let wanted = [
        text(&quote!(impl crate::command::table::ArgumentGroup for Selection)),
        text(&quote!(name: "rectangle", letter: ::core::option::Option::None)),
        text(&quote!(value_name: "x,y,w,h")),
        text(&quote!((
            "rectangle",
            crate::command::table::Given::given(&self.rectangle)
        ))),
        text(&quote!(fn first_given(&self))),
    ];
    let missing: Vec<_> = wanted.iter().filter(|want| !out.contains(*want)).collect();
    assert!(missing.is_empty(), "{missing:?} in {out}");
}

#[test]
fn local_files_stand_for_a_dry_run() {
    let write = |keys: Tokens| {
        expand(quote! {
            #[holst_command(group = Board, verb = "export", risk = Write, #keys
                about = "a", help = "b")]
            struct ExportArgs { #[argument(help = "c")] board: String }
        })
        .map(|tokens| tokens.to_string())
    };
    assert!(write(quote!(local_files,)).is_ok());
}

#[test]
fn an_argument_names_its_surfaces() {
    let out = expand(quote! {
        #[holst_command(group = Board, verb = "snap", risk = Read, about = "a", help = "b")]
        struct SnapArgs {
            #[argument(help = "c", surfaces = [Cli])] file: String,
            #[argument(help = "d")] board: String,
        }
    })
    .map(|tokens| tokens.to_string())
    .unwrap_or_default();
    let wanted = [
        text(&quote!(surfaces: &[crate::error::hint::Surface::Cli],)),
        text(&quote!(
            surfaces: &[crate::error::hint::Surface::Cli, crate::error::hint::Surface::Mcp],
        )),
    ];
    let missing: Vec<_> = wanted.iter().filter(|want| !out.contains(*want)).collect();
    assert!(missing.is_empty(), "{missing:?} in {out}");
}

#[test]
fn a_subcommand_and_the_kinds_it_limits_are_emitted() {
    let out = expand(quote! {
        #[holst_command(group = Object, verb = "add", risk = Write, about = "a", help = "b")]
        struct AddArgs {
            #[argument(help = "c", subcommand)] kind: Option<AddKind>,
            #[argument(help = "d", kinds = [shape, sticker_stack])] shape: Option<String>,
            #[argument(help = "e")] board: String,
            #[argument(help = "f")] dry_run: WriteMode,
        }
    })
    .map(|tokens| tokens.to_string())
    .unwrap_or_default();
    let wanted = [
        text(&quote!(subcommand: true, kinds: &[],)),
        text(&quote!(subcommand: false, kinds: &["shape", "sticker-stack"],)),
        text(&quote!(subcommand: false, kinds: &[],)),
    ];
    let missing: Vec<_> = wanted.iter().filter(|want| !out.contains(*want)).collect();
    assert!(missing.is_empty(), "{missing:?} in {out}");
}

#[test]
fn a_flattened_group_is_declared_alone() {
    let error = expand(quote! {
        #[holst_command(group = Board, risk = Read, about = "a", help = "b")]
        struct ShowArgs { #[argument(flatten, help = "c")] selection: Selection }
    })
    .err()
    .map(|error| error.to_string());
    assert_eq!(error.as_deref(), Some(FLATTEN_ALONE));
}

#[test]
fn a_group_does_not_hold_a_group() {
    let error = expand_group(quote! {
        struct Selection { #[argument(flatten)] inner: Inner }
    })
    .err()
    .map(|error| error.to_string());
    assert_eq!(error.as_deref(), Some(NO_NESTED_GROUP));
}

#[rstest]
#[case::write_without_dry_run(
    quote! {
        #[holst_command(group = Board, verb = "rename", risk = Write, about = "a", help = "b")]
        struct RenameArgs { #[argument(help = "c")] name: String }
    },
    "a command that changes something needs a `dry_run` field"
)]
#[case::destroy_without_confirm(
    quote! {
        #[holst_command(group = Board, verb = "rm", risk = Destroy, about = "a", help = "b")]
        struct RemoveArgs { #[argument(help = "c")] dry_run: WriteMode }
    },
    "`risk = Destroy` needs a `confirm` field"
)]
#[case::no_dry_run_on_mcp(
    quote! {
        #[holst_command(group = Auth, verb = "login", risk = Write, no_dry_run,
            about = "a", help = "b")]
        struct LoginArgs {}
    },
    "`no_dry_run` is for a command on the command line only: add `surfaces = [Cli]`"
)]
#[case::renamed_field(
    quote! {
        #[holst_command(group = Auth, risk = Read, about = "a", help = "b")]
        struct StatusArgs { #[serde(rename = "w")] #[argument(help = "c")] with: Vec<String> }
    },
    "`#[serde(rename)]` would make a name differ between the command line and MCP"
)]
#[case::renamed_struct(
    quote! {
        #[serde(rename_all = "camelCase")]
        #[holst_command(group = Auth, risk = Read, about = "a", help = "b")]
        struct StatusArgs {}
    },
    "`#[serde(rename_all)]` would make a name differ between the command line and MCP"
)]
#[case::flattened_field(
    quote! {
        #[holst_command(group = Auth, risk = Read, about = "a", help = "b")]
        struct StatusArgs { #[serde(flatten)] #[argument(help = "c")] part: Part }
    },
    "`#[serde(flatten)]` would make a name differ between the command line and MCP"
)]
#[case::one_letter_twice(
    quote! {
        #[holst_command(group = Board, risk = Read, about = "a", help = "b")]
        struct ListArgs {
            #[argument(letter = 'w', help = "c")] workspace: String,
            #[argument(letter = 'w', help = "d")] width: String,
        }
    },
    "the letter `-w` is given to both `workspace` and `width`"
)]
#[case::two_subcommands(
    quote! {
        #[holst_command(group = Object, risk = Read, about = "a", help = "b")]
        struct AddArgs {
            #[argument(help = "c", subcommand)] kind: String,
            #[argument(help = "d", subcommand)] shape: String,
        }
    },
    ONE_SUBCOMMAND
)]
#[case::kinds_without_a_subcommand(
    quote! {
        #[holst_command(group = Object, risk = Read, about = "a", help = "b")]
        struct AddArgs { #[argument(help = "c", kinds = [shape])] shape: String }
    },
    KINDS_NEED_SUBCOMMAND
)]
#[case::kinds_on_the_subcommand(
    quote! {
        #[holst_command(group = Object, risk = Read, about = "a", help = "b")]
        struct AddArgs { #[argument(help = "c", subcommand, kinds = [shape])] kind: String }
    },
    KINDS_NEED_SUBCOMMAND
)]
#[case::no_help(
    quote! {
        #[holst_command(group = Auth, risk = Read, about = "a", help = "b")]
        struct StatusArgs { with: Vec<String> }
    },
    "field `with` needs `#[argument(help = \"…\")]`"
)]
#[case::no_group(
    quote! {
        #[holst_command(risk = Read, about = "a", help = "b")]
        struct StatusArgs {}
    },
    "`#[holst_command(..)]` needs `group`"
)]
#[case::key_twice(
    quote! {
        #[holst_command(group = Auth, group = Board, risk = Read, about = "a", help = "b")]
        struct StatusArgs {}
    },
    "`group` is given twice"
)]
#[case::unknown_command_key(
    quote! {
        #[holst_command(group = Auth, risk = Read, about = "a", help = "b", hidden)]
        struct StatusArgs {}
    },
    COMMAND_KEYS
)]
#[case::unknown_argument_key(
    quote! {
        #[holst_command(group = Auth, risk = Read, about = "a", help = "b")]
        struct StatusArgs { #[argument(help = "c", short = 'w')] with: Vec<String> }
    },
    ARGUMENT_KEYS
)]
#[case::tuple_struct(
    quote! {
        #[holst_command(group = Auth, risk = Read, about = "a", help = "b")]
        struct StatusArgs(Vec<String>);
    },
    NAMED_FIELDS
)]
fn mistakes_are_compile_errors(#[case] item: Tokens, #[case] message: &str) {
    let error = expand(item).err().map(|error| error.to_string());
    assert_eq!(error.as_deref(), Some(message));
}

#[rstest]
#[case::dry_run_field("Write", quote!(#[argument(help = "d")] dry_run: WriteMode,), true)]
#[case::read_without_one("Read", quote!(), false)]
fn changes_are_emitted_where_there_is_a_dry_run(
    #[case] risk: &str,
    #[case] field: Tokens,
    #[case] emitted: bool,
) {
    let risk = quote::format_ident!("{risk}");
    let out = expand(quote! {
        #[holst_command(group = Board, verb = "move", risk = #risk, about = "a", help = "b")]
        struct MoveArgs {
            #[argument(help = "c")]
            board: String,
            #field
        }
    })
    .map(|tokens| tokens.to_string())
    .unwrap_or_default();
    let wanted = text(&quote!(
        impl crate::command::table::Changes for MoveArgs
    ));
    assert_eq!(out.contains(&wanted), emitted, "{out}");
}
