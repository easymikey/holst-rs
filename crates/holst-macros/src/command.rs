//! The `CommandArgs` derive: a command's table entry from its arguments struct.

use proc_macro2::{Span, TokenStream as Tokens, TokenTree};
use quote::{ToTokens, quote};
use syn::{
    Attribute,
    Expr,
    Field,
    Fields,
    Ident,
    ItemStruct,
    LitChar,
    LitStr,
    Meta,
    Token,
    Type,
    bracketed,
    ext::IdentExt as _,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

pub(crate) const COMMAND_KEYS: &str = "expected `group = <Group>`, `verb = \"…\"`, `risk = <Risk>`, \
                                       `block = <HelpBlock>`, `about = \"…\"`, `help = \"…\"`, \
                                       `example = \"…\"`, `surfaces = [..]`, `no_dry_run` or `local_files`";
pub(crate) const ARGUMENT_KEYS: &str = "expected `help = \"…\"`, `value_name = \"…\"`, \
                                        `letter = 'x'`, `tool_doc = \"…\"`, `surfaces = [..]`, \
                                        `positional`, `subcommand`, `kinds = [..]` or `flatten`";
pub(crate) const NAMED_FIELDS: &str = "`CommandArgs` takes a struct with named fields";
pub(crate) const FLATTEN_ALONE: &str =
    "`flatten` takes no other key: the group's own fields carry their texts";
pub(crate) const NO_NESTED_GROUP: &str = "an argument group cannot hold a group";
pub(crate) const ONE_SUBCOMMAND: &str = "a command takes one `subcommand` field";
pub(crate) const KINDS_NEED_SUBCOMMAND: &str = "`kinds` limits a field to values of the command's `subcommand` field: give the command one, on another field";
const COMMAND_ATTRIBUTE: &str = "holst_command";
const COMMAND_WORDS: [&str; 10] = [
    "group",
    "verb",
    "risk",
    "block",
    "about",
    "help",
    EXAMPLE,
    SURFACES,
    NO_DRY_RUN,
    LOCAL_FILES,
];
const ARGUMENT_ATTRIBUTE: &str = "argument";
const ARGUMENT_WORDS: [&str; 9] = [
    "help",
    "value_name",
    "letter",
    "tool_doc",
    "positional",
    SURFACES,
    SUBCOMMAND,
    KINDS,
    FLATTEN,
];
const EXAMPLE: &str = "example";
const FLATTEN: &str = "flatten";
const NO_DRY_RUN: &str = "no_dry_run";
const LOCAL_FILES: &str = "local_files";
const SURFACES: &str = "surfaces";
const SUBCOMMAND: &str = "subcommand";
const KINDS: &str = "kinds";
const RENAMES: [&str; 4] = ["rename", "rename_all", "alias", FLATTEN];

/// One `word` or `word = value` of an attribute.
struct Key {
    word: Ident,
    value: Option<Expr>,
}

impl Parse for Key {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let word = input.call(Ident::parse_any)?;
        let value = if input.peek(Token![=]) {
            Some(assigned(input)?)
        } else {
            None
        };
        Ok(Self { word, value })
    }
}

fn assigned(input: ParseStream<'_>) -> syn::Result<Expr> {
    input.parse::<Token![=]>()?;
    input.parse()
}

/// `[A, B]`: the surfaces of a command.
struct Listed(Vec<Ident>);

impl Parse for Listed {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let content;
        bracketed!(content in input);
        Punctuated::<Ident, Token![,]>::parse_terminated(&content)
            .map(|list| Listed(list.into_iter().collect()))
    }
}

/// The keys of every `#[name(..)]` among `attributes`, each word known and
/// given once (but `example`).
fn keys(attributes: &[Attribute], name: &str, known: &[&str]) -> syn::Result<Vec<Key>> {
    let keys = attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident(name))
        .map(|attribute| {
            attribute.parse_args_with(Punctuated::<Key, Token![,]>::parse_terminated)
        })
        .try_fold(Vec::new(), |mut all, parsed| {
            all.extend(parsed?);
            Ok::<_, syn::Error>(all)
        })?;
    let expected = if name == COMMAND_ATTRIBUTE {
        COMMAND_KEYS
    } else {
        ARGUMENT_KEYS
    };
    let unknown = keys
        .iter()
        .find(|key| !known.iter().any(|word| key.word == word));
    let twice = keys.iter().enumerate().find(|(position, key)| {
        key.word != EXAMPLE
            && keys
                .iter()
                .take(*position)
                .any(|earlier| earlier.word == key.word)
    });
    match (unknown, twice) {
        (Some(key), _) => Err(syn::Error::new(key.word.span(), expected)),
        (None, Some((_, key))) => Err(syn::Error::new(
            key.word.span(),
            format!("`{}` is given twice", key.word),
        )),
        (None, None) => Ok(keys),
    }
}

/// Returns the value of every key named `word`, read as `T`.
fn values<T: Parse>(keys: &[Key], word: &str) -> syn::Result<Vec<T>> {
    keys.iter()
        .filter(|key| key.word == word)
        .map(|key| {
            key.value
                .as_ref()
                .ok_or_else(|| {
                    syn::Error::new(key.word.span(), format!("`{word}` needs a value"))
                })
                .and_then(|value| syn::parse2(value.to_token_stream()))
        })
        .collect()
}

fn value<T: Parse>(keys: &[Key], word: &str) -> syn::Result<Option<T>> {
    values(keys, word).map(|found| found.into_iter().next())
}

fn required<T>(value: Option<T>, word: &str) -> syn::Result<T> {
    value.ok_or_else(|| {
        syn::Error::new(
            Span::call_site(),
            format!("`#[{COMMAND_ATTRIBUTE}(..)]` needs `{word}`"),
        )
    })
}

/// Refuses the serde attributes that would make a field's name differ on the
/// command line, in JSON and on MCP. `#[serde(flatten)]` is allowed on a field
/// that is an argument group (`flattened`): its names join the command's own.
fn refuse_renames(attributes: &[Attribute], flattened: bool) -> syn::Result<()> {
    attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("serde"))
        .filter_map(|attribute| match &attribute.meta {
            Meta::List(list) => Some(list.tokens.clone()),
            Meta::Path(_) | Meta::NameValue(_) => None,
        })
        .flatten()
        .find_map(|token| match token {
            TokenTree::Ident(word)
                if RENAMES.iter().any(|rename| word == rename)
                    && !(flattened && word == FLATTEN) =>
            {
                Some(word)
            }
            TokenTree::Ident(_)
            | TokenTree::Group(_)
            | TokenTree::Punct(_)
            | TokenTree::Literal(_) => None,
        })
        .map_or(Ok(()), |word| {
            Err(syn::Error::new(
                word.span(),
                format!(
                    "`#[serde({word})]` would make a name differ between the command line and MCP"
                ),
            ))
        })
}

/// The surfaces a key list names, both when it names none.
fn surfaces(keys: &[Key]) -> syn::Result<Vec<Ident>> {
    let ident = |word: &str| Ident::new(word, Span::call_site());
    Ok(value::<Listed>(keys, SURFACES)?.map_or_else(
        || vec![ident("Cli"), ident("Mcp")],
        |Listed(surfaces)| surfaces,
    ))
}

struct ArgumentField {
    ident: Ident,
    name: String,
    ty: Type,
    letter: Option<LitChar>,
    positional: bool,
    value_name: LitStr,
    help: LitStr,
    tool_doc: Option<LitStr>,
    surfaces: Vec<Ident>,
    /// `#[argument(subcommand)]`: its value picks the command's kind.
    subcommand: bool,
    /// `#[argument(kinds = [..])]`: the subcommand values it is for.
    kinds: Vec<Ident>,
}

/// A field of an arguments struct: one argument, or a group of them.
enum Member {
    Plain(ArgumentField),
    /// `#[argument(flatten)]`: the type's [`ArgumentGroup`] arguments.
    Group(Type),
}

impl Member {
    fn of(field: &Field) -> syn::Result<Self> {
        let keys = keys(&field.attrs, ARGUMENT_ATTRIBUTE, &ARGUMENT_WORDS)?;
        let flatten = keys.iter().find(|key| key.word == FLATTEN);
        refuse_renames(&field.attrs, flatten.is_some())?;
        match flatten {
            Some(key) if keys.len() > 1 => {
                Err(syn::Error::new(key.word.span(), FLATTEN_ALONE))
            }
            Some(_) => Ok(Self::Group(field.ty.clone())),
            None => ArgumentField::of(field, &keys).map(Self::Plain),
        }
    }
}

/// The named fields of an arguments struct.
fn named_fields(item: &ItemStruct) -> syn::Result<Vec<Member>> {
    match &item.fields {
        Fields::Named(named) => named.named.iter().map(Member::of).collect(),
        Fields::Unnamed(_) | Fields::Unit => {
            Err(syn::Error::new(item.ident.span(), NAMED_FIELDS))
        }
    }
}

impl ArgumentField {
    fn of(field: &Field, keys: &[Key]) -> syn::Result<Self> {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new(Span::call_site(), NAMED_FIELDS))?;
        let name = ident.unraw().to_string();
        let help = value(keys, "help")?.ok_or_else(|| {
            syn::Error::new(
                ident.span(),
                format!("field `{name}` needs `#[{ARGUMENT_ATTRIBUTE}(help = \"…\")]`"),
            )
        })?;
        let value_name = value(keys, "value_name")?
            .unwrap_or_else(|| LitStr::new(&name, ident.span()));
        let letter = value(keys, "letter")?;
        let tool_doc = value(keys, "tool_doc")?;
        let positional = keys.iter().any(|key| key.word == "positional");
        let surfaces = surfaces(keys)?;
        let subcommand = keys.iter().any(|key| key.word == SUBCOMMAND);
        let kinds =
            value::<Listed>(keys, KINDS)?.map_or_else(Vec::new, |Listed(list)| list);
        Ok(Self {
            ident: ident.clone(),
            name,
            ty: field.ty.clone(),
            letter,
            positional,
            value_name,
            help,
            tool_doc,
            surfaces,
            subcommand,
            kinds,
        })
    }

    /// `("name", given)`: whether the field holds a value the caller gave.
    fn given(&self) -> Tokens {
        let (name, ident) = (&self.name, &self.ident);
        quote!((#name, crate::command::table::Given::given(&self.#ident)))
    }

    fn emit(&self) -> Tokens {
        let Self {
            name,
            ty,
            letter,
            positional,
            value_name,
            help,
            tool_doc,
            surfaces,
            subcommand,
            kinds,
            ..
        } = self;
        let kinds = kinds.iter().map(|kind| kind.to_string().replace('_', "-"));
        let (letter, tool_doc) =
            (optional(letter.as_ref()), optional(tool_doc.as_ref()));
        quote! {
            crate::command::table::Argument {
                name: #name,
                letter: #letter,
                kind: <#ty as crate::command::table::ArgumentValue>::KIND,
                required: <#ty as crate::command::table::ArgumentValue>::REQUIRED,
                positional: #positional,
                value_name: #value_name,
                help: #help,
                tool_doc: #tool_doc,
                surfaces: &[#(crate::error::hint::Surface::#surfaces),*],
                subcommand: #subcommand,
                kinds: &[#(#kinds),*],
            }
        }
    }
}

fn optional<T: ToTokens>(value: Option<&T>) -> Tokens {
    value.map_or_else(
        || quote!(::core::option::Option::None),
        |value| quote!(::core::option::Option::Some(#value)),
    )
}

struct CommandItem {
    name: Ident,
    group: Ident,
    verb: Option<LitStr>,
    risk: Ident,
    block: Ident,
    about: LitStr,
    help: LitStr,
    examples: Vec<LitStr>,
    surfaces: Vec<Ident>,
    no_dry_run: bool,
    /// Writes new files on this computer only, never the board: no `dry_run`.
    local_files: bool,
    members: Vec<Member>,
}

impl CommandItem {
    fn of(item: &ItemStruct) -> syn::Result<Self> {
        refuse_renames(&item.attrs, false)?;
        let members = named_fields(item)?;
        let keys = keys(&item.attrs, COMMAND_ATTRIBUTE, &COMMAND_WORDS)?;
        let ident = |word: &str| Ident::new(word, Span::call_site());
        Ok(Self {
            name: item.ident.clone(),
            group: required(value(&keys, "group")?, "group")?,
            verb: value(&keys, "verb")?,
            risk: required(value(&keys, "risk")?, "risk")?,
            block: value(&keys, "block")?.unwrap_or_else(|| ident("Commands")),
            about: required(value(&keys, "about")?, "about")?,
            help: required(value(&keys, "help")?, "help")?,
            examples: values(&keys, EXAMPLE)?,
            surfaces: surfaces(&keys)?,
            no_dry_run: keys.iter().any(|key| key.word == NO_DRY_RUN),
            local_files: keys.iter().any(|key| key.word == LOCAL_FILES),
            members,
        })
    }

    /// The arguments the struct declares itself, not those of its groups.
    fn own(&self) -> impl Iterator<Item = &ArgumentField> {
        self.members.iter().filter_map(|member| match member {
            Member::Plain(argument) => Some(argument),
            Member::Group(_) => None,
        })
    }

    fn has(&self, field: &str) -> bool {
        self.own().any(|argument| argument.name == field)
    }

    /// One `subcommand` field at most, and `kinds` only beside one.
    fn check_subcommand(&self) -> syn::Result<()> {
        let subcommands = self.own().filter(|argument| argument.subcommand).count();
        let limited = self.own().any(|argument| !argument.kinds.is_empty());
        let error = |text: &str| Err(syn::Error::new(self.name.span(), text));
        match (subcommands, limited) {
            (2.., _) => error(ONE_SUBCOMMAND),
            (0, true) => error(KINDS_NEED_SUBCOMMAND),
            (0 | 1, _) => {
                if self
                    .own()
                    .any(|argument| argument.subcommand && !argument.kinds.is_empty())
                {
                    error(KINDS_NEED_SUBCOMMAND)
                } else {
                    Ok(())
                }
            }
        }
    }

    fn check(&self) -> syn::Result<()> {
        let risk = self.risk.to_string();
        let cli_only = matches!(self.surfaces.as_slice(), [only] if *only == "Cli");
        let error = |text: &str| Err(syn::Error::new(self.risk.span(), text));
        let doubled = self.own().enumerate().find_map(|(position, argument)| {
            let letter = argument.letter.as_ref()?.value();
            self.own()
                .take(position)
                .find(|earlier| {
                    earlier
                        .letter
                        .as_ref()
                        .is_some_and(|it| it.value() == letter)
                })
                .map(|earlier| (letter, &earlier.name, &argument.name))
        });
        match (risk.as_str(), self.no_dry_run, cli_only) {
            (_, true, false) => error(
                "`no_dry_run` is for a command on the command line only: add `surfaces = [Cli]`",
            ),
            ("Write" | "Destroy", false, _)
                if !self.has("dry_run") && !self.local_files =>
            {
                error("a command that changes something needs a `dry_run` field")
            }
            ("Destroy", _, _) if !self.has("confirm") => {
                error("`risk = Destroy` needs a `confirm` field")
            }
            _ => doubled.map_or(Ok(()), |(letter, first, second)| {
                Err(syn::Error::new(
                    self.name.span(),
                    format!("the letter `-{letter}` is given to both `{first}` and `{second}`"),
                ))
            }),
        }
    }

    fn emit(&self) -> Tokens {
        let Self {
            name,
            group,
            risk,
            block,
            about,
            help,
            examples,
            surfaces,
            ..
        } = self;
        let verb = optional(self.verb.as_ref());
        let arguments = arguments(&self.members);
        let changes = self.has("dry_run").then(|| {
            quote! {
                impl crate::command::table::Changes for #name {
                    fn mode(&self) -> crate::effect::board_write::WriteMode {
                        self.dry_run
                    }
                }
            }
        });
        quote! {
            #changes
            impl crate::command::table::CommandArgs for #name {
                const COMMAND: crate::command::table::Command = crate::command::table::Command {
                    group: crate::command::table::CommandGroup::#group,
                    verb: #verb,
                    about: #about,
                    help: #help,
                    examples: &[#(#examples),*],
                    block: crate::command::table::HelpBlock::#block,
                    risk: crate::command::table::CommandRisk::#risk,
                    surfaces: &[#(crate::error::hint::Surface::#surfaces),*],
                    arguments: #arguments,
                };
            }
        }
    }
}

/// The `&'static [Argument]` of `members`: a literal slice, or, when a group is
/// flattened in, the members joined in field order at compile time.
fn arguments(members: &[Member]) -> Tokens {
    let own: Option<Vec<Tokens>> = members
        .iter()
        .map(|member| match member {
            Member::Plain(argument) => Some(argument.emit()),
            Member::Group(_) => None,
        })
        .collect();
    own.map_or_else(
        || {
            let parts = members.iter().map(|member| match member {
                Member::Plain(argument) => {
                    let argument = argument.emit();
                    quote!(&[#argument])
                }
                Member::Group(ty) => {
                    quote!(<#ty as crate::command::table::ArgumentGroup>::ARGUMENTS)
                }
            });
            quote! {{
                const PARTS: &[&[crate::command::table::Argument]] = &[#(#parts),*];
                const JOINED: [crate::command::table::Argument;
                    crate::command::table::joined_len(PARTS)] =
                    crate::command::table::joined(PARTS);
                &JOINED
            }}
        },
        |own| quote!(&[#(#own),*]),
    )
}

/// Expands `#[derive(CommandArgs)]` on `item`.
pub(crate) fn expand(item: Tokens) -> syn::Result<Tokens> {
    let command = CommandItem::of(&syn::parse2(item)?)?;
    command.check()?;
    command.check_subcommand()?;
    Ok(command.emit())
}

/// Expands `#[derive(ArgumentGroup)]` on `item`: arguments several commands
/// share, declared once.
pub(crate) fn expand_group(item: Tokens) -> syn::Result<Tokens> {
    let item: ItemStruct = syn::parse2(item)?;
    refuse_renames(&item.attrs, false)?;
    let name = &item.ident;
    let members = named_fields(&item)?;
    let nested = members
        .iter()
        .any(|member| matches!(member, Member::Group(_)));
    (!nested)
        .then_some(())
        .ok_or_else(|| syn::Error::new(name.span(), NO_NESTED_GROUP))?;
    let arguments = arguments(&members);
    let given = members.iter().filter_map(|member| match member {
        Member::Plain(argument) => Some(argument.given()),
        Member::Group(_) => None,
    });
    Ok(quote! {
        impl crate::command::table::ArgumentGroup for #name {
            const ARGUMENTS: &'static [crate::command::table::Argument] = #arguments;

            fn first_given(&self) -> ::core::option::Option<&'static str> {
                [#(#given),*]
                    .into_iter()
                    .find_map(|(name, given)| given.then_some(name))
            }
        }
    })
}

#[cfg(test)] mod tests;
