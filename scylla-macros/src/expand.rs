//! The expansion of `authz!`: the parse, then the checks of the names.

use std::collections::{HashMap, HashSet};

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, Ident, Path, Result};

use crate::code::{code, upper};
use crate::parse::{Block, Expr, Head, Input, ItemKind, Term};

/// The words of the SpiceDB schema language that cannot be names.
const RESERVED: [&str; 5] = ["caveat", "definition", "nil", "permission", "relation"];

/// Expands the input. The errors of one block come together.
pub(crate) fn expand(input: TokenStream) -> Result<TokenStream> {
    let input: Input = syn::parse2(input)?;
    let mut output = TokenStream::new();
    for block in &input.blocks {
        output.extend(block_code(block)?);
    }
    Ok(output)
}

/// What the names of a block refer to.
struct Scope<'a> {
    /// The name of the definition or of the holder, for the error messages.
    label: String,
    /// The relations of the block, with the Rust type of their subjects.
    relations: HashMap<String, &'a Path>,
    /// The permissions of the block.
    permissions: HashSet<String>,
    /// The Rust type of the definition that an extension adds to.
    base: Option<&'a Path>,
}

impl Scope<'_> {
    /// Returns `true` if the block declares this name.
    fn has(&self, name: &str) -> bool {
        self.relations.contains_key(name) || self.permissions.contains(name)
    }
}

/// The errors of a block, reported together.
#[derive(Default)]
struct Errors(Option<Error>);

impl Errors {
    /// Adds an error.
    fn push(&mut self, error: Error) {
        match &mut self.0 {
            Some(all) => all.combine(error),
            None => self.0 = Some(error),
        }
    }

    /// Returns all the errors as one, if there is one.
    fn finish(self) -> Result<()> {
        self.0.map_or(Ok(()), Err)
    }
}

/// Checks the names of a block, then makes its code.
fn block_code(block: &Block) -> Result<TokenStream> {
    // Check the name of a definition. An extension has the name of its base.
    let mut errors = Errors::default();
    let (label, base) = match &block.head {
        Head::Definition { name, .. } => {
            check_name(name, &mut errors);
            (name.to_string(), None)
        }
        Head::Extension { base, holder } => (holder.to_string(), Some(base)),
    };

    // Collect the names of the block, and refuse a bad name or a name declared twice.
    let mut scope = Scope {
        label,
        relations: HashMap::new(),
        permissions: HashSet::new(),
        base,
    };
    for item in &block.items {
        check_name(&item.name, &mut errors);
        let name = item.name.to_string();
        if scope.has(&name) {
            errors.push(Error::new(
                item.name.span(),
                format!("`{name}` is declared twice"),
            ));
        }
        match &item.kind {
            ItemKind::Relation(subject) => {
                scope.relations.insert(name, subject);
            }
            ItemKind::Permission(_) => {
                scope.permissions.insert(name);
            }
        }
    }

    // Check the names in the permissions, then make the code if there is no error.
    let mut checks = Vec::new();
    for item in &block.items {
        if let ItemKind::Permission(expr) = &item.kind {
            references(expr, &scope, &mut checks, &mut errors);
        }
    }
    errors.finish()?;
    Ok(code(block, &checks))
}

/// Walks the terms of an expression: an error for a wrong name of the block, and a check by
/// the compiler for each name on another type.
fn references(expr: &Expr, scope: &Scope<'_>, checks: &mut Vec<TokenStream>, errors: &mut Errors) {
    let rest = expr.rest.iter().map(|(_, term)| term);
    for term in std::iter::once(&expr.first).chain(rest) {
        match term {
            Term::Nil => {}
            Term::Group(inner) => references(inner, scope, checks, errors),

            // A name of the block is correct. In an extension, another name is on the base.
            Term::Name(name) if scope.has(&name.to_string()) => {}
            Term::Name(name) => match scope.base {
                Some(base) => checks.push(constant(base, name)),
                None => errors.push(Error::new(
                    name.span(),
                    format!(
                        "`{name}` is not a relation or a permission of `{}`",
                        scope.label
                    ),
                )),
            },

            // An arrow starts from a relation, and its target is on the subject type.
            Term::Arrow(relation, target) => {
                let text = relation.to_string();
                if let Some(subject) = scope.relations.get(&text) {
                    checks.push(constant(subject, target));
                } else if scope.permissions.contains(&text) {
                    errors.push(Error::new(
                        relation.span(),
                        format!("`{relation}` is a permission: an arrow starts from a relation"),
                    ));
                } else if let Some(base) = scope.base {
                    checks.push(constant(base, relation));
                } else {
                    errors.push(Error::new(
                        relation.span(),
                        format!("`{relation}` is not a relation of `{}`", scope.label),
                    ));
                }
            }
        }
    }
}

/// Makes `let _ = <Type>::NAME;`: the compiler checks that the type has this name.
fn constant(ty: &Path, name: &Ident) -> TokenStream {
    let name = upper(name);
    quote! { let _ = <#ty>::#name; }
}

/// Checks a SpiceDB name: 3 to 64 characters of `a-z`, `0-9` and `_`, a letter first, no `_`
/// last, and not a word of the language.
fn check_name(name: &Ident, errors: &mut Errors) {
    // Check the length and the characters.
    let text = name.to_string();
    let valid = (3..=64).contains(&text.len())
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && !text.ends_with('_')
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');

    // Refuse a bad name, or a word of the language.
    if !valid {
        errors.push(Error::new(
            name.span(),
            format!(
                "`{text}` is not a SpiceDB name: use 3 to 64 characters of a-z, 0-9 and _, \
                 a letter first and no _ last"
            ),
        ));
    } else if RESERVED.contains(&text.as_str()) {
        errors.push(Error::new(
            name.span(),
            format!("`{text}` is a word of the SpiceDB schema language"),
        ));
    }
}
