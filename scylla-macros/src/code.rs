//! The generated code of `authz!`: the schema part and the typed constants.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Ident, Path};

use crate::parse::{Block, Expr, Head, ItemKind, Op, Term};

/// Returns the constant of a schema name: `act_for` gives `ACT_FOR`.
pub(crate) fn upper(name: &Ident) -> Ident {
    Ident::new(&name.to_string().to_uppercase(), name.span())
}

/// Returns an expression in the SpiceDB schema language.
fn render(expr: &Expr) -> String {
    let mut text = render_term(&expr.first);
    for (op, term) in &expr.rest {
        text.push_str(match op {
            Op::Union => " + ",
            Op::Intersection => " & ",
            Op::Exclusion => " - ",
        });
        text.push_str(&render_term(term));
    }
    text
}

/// Returns a term in the SpiceDB schema language.
fn render_term(term: &Term) -> String {
    match term {
        Term::Nil => "nil".to_owned(),
        Term::Name(name) => name.to_string(),
        Term::Arrow(relation, target) => format!("{relation}->{target}"),
        Term::Group(inner) => format!("({})", render(inner)),
    }
}

/// Returns a path as Rust text, with no generic arguments.
fn path_text(path: &Path) -> String {
    let segments: Vec<String> = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    segments.join("::")
}

/// Returns the doc comments of an item, then one more paragraph.
fn docs(attrs: &[Attribute], line: &str) -> TokenStream {
    let line = format!(" {line}");
    if attrs.is_empty() {
        quote! { #[doc = #line] }
    } else {
        quote! { #(#attrs)* #[doc = ""] #[doc = #line] }
    }
}

/// Makes the code of a block whose names are correct.
pub(crate) fn code(block: &Block, checks: &[TokenStream]) -> TokenStream {
    // Find the resource of the constants: the type of the definition, or the extended type.
    let authz = quote!(::scylla_domain::authz);
    let resource = match &block.head {
        Head::Definition { ty, .. } => ty,
        Head::Extension { base, .. } => base,
    };

    // Make the schema entry and the typed constant of each relation and permission.
    let mut relations = Vec::new();
    let mut permissions = Vec::new();
    let mut constants = Vec::new();
    for item in &block.items {
        let name = item.name.to_string();
        let (ty, schema) = match &item.kind {
            ItemKind::Relation(subject) => {
                relations.push(quote! { (#name, <#subject as #authz::Resource>::NAME) });
                let ty = quote! { #authz::Relation<#resource, #subject> };
                (ty, format!("relation {name}: {}", path_text(subject)))
            }
            ItemKind::Permission(expr) => {
                let expression = render(expr);
                let schema = format!("permission {name} = {expression}");
                permissions.push(quote! { (#name, #expression) });
                (quote! { #authz::Permission<#resource> }, schema)
            }
        };
        let docs = docs(&item.attrs, &format!("Schema: `{schema}`."));
        let constant = upper(&item.name);
        constants.push(quote! {
            #docs
            pub const #constant: #ty = <#ty>::new(#name);
        });
    }

    // Make the code of the block, with the checks of the names in `Model::part`.
    let part = quote! {
        ::std::vec![#(#relations),*],
        ::std::vec![#(#permissions),*],
    };
    let attrs = &block.attrs;
    match &block.head {
        // A definition: `Resource` and `Model` for the type, and the constants on the type.
        Head::Definition { name, ty } => {
            let name = name.to_string();
            quote! {
                impl #authz::Resource for #ty {
                    const NAME: &'static str = #name;
                }

                impl #authz::Model for #ty {
                    fn part() -> #authz::Part {
                        #(#checks)*
                        #authz::Part::definition(#name, #part)
                    }
                }

                #(#attrs)*
                #[allow(dead_code)]
                impl #ty {
                    #(#constants)*
                }
            }
        }
        // An extension: a holder struct with `Model` and the constants.
        Head::Extension { base, holder } => {
            let docs = docs(
                attrs,
                &format!("It extends the definition of `{}`.", path_text(base)),
            );
            quote! {
                #docs
                #[derive(Clone, Copy, Debug)]
                #[allow(dead_code)]
                pub struct #holder;

                impl #authz::Model for #holder {
                    fn part() -> #authz::Part {
                        #(#checks)*
                        #authz::Part::extension(<#base as #authz::Resource>::NAME, #part)
                    }
                }

                #[allow(dead_code)]
                impl #holder {
                    #(#constants)*
                }
            }
        }
    }
}
