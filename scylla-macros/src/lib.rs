//! The procedural macros of Scylla: `authz!`, the permission model in the SpiceDB schema language.

mod code;
mod expand;
mod parse;
#[cfg(test)]
mod tests;

use proc_macro::TokenStream;

/// Declares a piece of the permission model: a SpiceDB definition for a Rust type, or more
/// relations and permissions for a definition. Use it as `scylla_domain::authz!`.
///
/// ```text
/// authz! {
///     /// The permission model of the projects.
///     definition project for Project {
///         /// The actor that owns the project.
///         relation owner: Actor;
///         /// Delete the project.
///         permission delete = owner->act_for;
///     }
/// }
///
/// authz! {
///     /// An auditor of the projects.
///     extend Project as ProjectAudit {
///         /// An actor that audits the project.
///         relation auditor: Actor;
///         /// Read the audit log.
///         permission audit = auditor->act_for + delete;
///     }
/// }
/// ```
///
/// - `definition <name> for <Type>` implements `Resource` and `Model` for `Type`, and adds one
///   constant to `Type` for each relation and permission: `Project::OWNER`, `Project::DELETE`.
/// - `extend <Type> as <Holder>` makes the unit struct `Holder`, with `Model` and the
///   constants of the new names: `ProjectAudit::AUDITOR`. The definition of `Type` keeps them.
/// - A relation has one subject type, a Rust type with `Resource`.
/// - A permission uses `+`, `&`, `-`, `->`, parentheses and `nil`. A name in it is a relation
///   or a permission of the definition. An arrow starts from a relation, and the compiler finds
///   its right side on the subject type.
#[proc_macro]
pub fn authz(input: TokenStream) -> TokenStream {
    expand::expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
