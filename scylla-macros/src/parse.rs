//! The input of `authz!`: definitions and extensions, in the SpiceDB schema language.

use syn::parse::{Parse, ParseStream};
use syn::{braced, parenthesized, token, Attribute, Error, Ident, Path, Result, Token};

/// The input: one or more blocks.
pub(crate) struct Input {
    /// The blocks, in order.
    pub(crate) blocks: Vec<Block>,
}

/// A definition, or an extension of a definition.
pub(crate) struct Block {
    /// The doc comments of the block.
    pub(crate) attrs: Vec<Attribute>,
    /// What the block declares.
    pub(crate) head: Head,
    /// The relations and the permissions.
    pub(crate) items: Vec<Item>,
}

/// The head of a block.
pub(crate) enum Head {
    /// `definition <name> for <Type>`: a new definition for a Rust type.
    Definition {
        /// The name of the definition.
        name: Ident,
        /// The Rust type of the ids of its objects.
        ty: Path,
    },
    /// `extend <Type> as <Holder>`: more relations and permissions for a definition.
    Extension {
        /// The Rust type of the definition.
        base: Path,
        /// The unit struct that holds the new constants.
        holder: Ident,
    },
}

/// A relation or a permission.
pub(crate) struct Item {
    /// The doc comments of the item.
    pub(crate) attrs: Vec<Attribute>,
    /// The name in the schema.
    pub(crate) name: Ident,
    /// The relation or the permission.
    pub(crate) kind: ItemKind,
}

/// The kind of an item.
pub(crate) enum ItemKind {
    /// `relation <name>: <Type>;`, with the Rust type of the subjects.
    Relation(Path),
    /// `permission <name> = <expression>;`
    Permission(Expr),
}

/// A permission expression: terms joined by operators, from left to right.
pub(crate) struct Expr {
    /// The first term.
    pub(crate) first: Term,
    /// The next terms, each with the operator before it.
    pub(crate) rest: Vec<(Op, Term)>,
}

/// An operator of a permission expression.
#[derive(Clone, Copy)]
pub(crate) enum Op {
    /// `+`
    Union,
    /// `&`
    Intersection,
    /// `-`
    Exclusion,
}

/// A term of a permission expression.
pub(crate) enum Term {
    /// `nil`: no subject.
    Nil,
    /// A relation or a permission of the definition.
    Name(Ident),
    /// `<relation>-><name>`: the name on each subject of the relation.
    Arrow(Ident, Ident),
    /// An expression in parentheses.
    Group(Box<Expr>),
}

impl Parse for Input {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut blocks = Vec::new();
        while !input.is_empty() {
            blocks.push(input.parse()?);
        }
        Ok(Self { blocks })
    }
}

impl Parse for Block {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let keyword: Ident = input.parse()?;
        let head = if keyword == "definition" {
            let name = input.parse()?;
            input.parse::<Token![for]>()?;
            Head::Definition {
                name,
                ty: input.parse()?,
            }
        } else if keyword == "extend" {
            let base = input.parse()?;
            input.parse::<Token![as]>()?;
            Head::Extension {
                base,
                holder: input.parse()?,
            }
        } else {
            return Err(Error::new(
                keyword.span(),
                "expected `definition` or `extend`",
            ));
        };
        let content;
        braced!(content in input);
        let mut items = Vec::new();
        while !content.is_empty() {
            items.push(content.parse()?);
        }
        Ok(Self { attrs, head, items })
    }
}

impl Parse for Item {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let keyword: Ident = input.parse()?;
        let name = input.parse()?;
        let kind = if keyword == "relation" {
            input.parse::<Token![:]>()?;
            ItemKind::Relation(input.parse()?)
        } else if keyword == "permission" {
            input.parse::<Token![=]>()?;
            ItemKind::Permission(input.parse()?)
        } else {
            return Err(Error::new(
                keyword.span(),
                "expected `relation` or `permission`",
            ));
        };
        input.parse::<Token![;]>()?;
        Ok(Self { attrs, name, kind })
    }
}

impl Parse for Expr {
    fn parse(input: ParseStream) -> Result<Self> {
        let first = input.parse()?;
        let mut rest = Vec::new();
        loop {
            let op = if input.peek(Token![+]) {
                input.parse::<Token![+]>()?;
                Op::Union
            } else if input.peek(Token![&]) {
                input.parse::<Token![&]>()?;
                Op::Intersection
            } else if input.peek(Token![->]) {
                return Err(input.error("an arrow starts from the name of a relation"));
            } else if input.peek(Token![-]) {
                input.parse::<Token![-]>()?;
                Op::Exclusion
            } else {
                break;
            };
            rest.push((op, input.parse()?));
        }
        Ok(Self { first, rest })
    }
}

impl Parse for Term {
    fn parse(input: ParseStream) -> Result<Self> {
        if input.peek(token::Paren) {
            let content;
            parenthesized!(content in input);
            return Ok(Self::Group(Box::new(content.parse()?)));
        }
        let name: Ident = input.parse()?;
        if name == "nil" {
            return Ok(Self::Nil);
        }
        if input.peek(Token![->]) {
            input.parse::<Token![->]>()?;
            return Ok(Self::Arrow(name, input.parse()?));
        }
        Ok(Self::Name(name))
    }
}
