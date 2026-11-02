//! The SpiceDB schema of an edition: the definitions of the plugins, with their extensions.

use std::fmt;

/// A piece of the permission model that a plugin registers. `authz!` implements it.
pub trait Model {
    /// Returns the piece.
    fn part() -> Part;
}

/// A definition, or the relations and permissions that an extension adds to a definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    /// The name of the definition.
    definition: &'static str,
    /// `true` if the part adds to the definition of another part.
    extension: bool,
    /// The relations: name and subject definition.
    relations: Vec<(&'static str, &'static str)>,
    /// The permissions: name and expression.
    permissions: Vec<(&'static str, &'static str)>,
}

impl Part {
    /// Makes a definition. Only `authz!` calls it.
    #[doc(hidden)]
    pub fn definition(
        name: &'static str,
        relations: Vec<(&'static str, &'static str)>,
        permissions: Vec<(&'static str, &'static str)>,
    ) -> Self {
        Self {
            definition: name,
            extension: false,
            relations,
            permissions,
        }
    }

    /// Makes an extension of a definition. Only `authz!` calls it.
    #[doc(hidden)]
    pub fn extension(
        definition: &'static str,
        relations: Vec<(&'static str, &'static str)>,
        permissions: Vec<(&'static str, &'static str)>,
    ) -> Self {
        Self {
            definition,
            extension: true,
            relations,
            permissions,
        }
    }

    /// Returns the name of the definition.
    pub fn definition_name(&self) -> &'static str {
        self.definition
    }

    /// Returns the names of the relations and the permissions.
    fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.relations
            .iter()
            .chain(&self.permissions)
            .map(|(name, _)| *name)
    }
}

/// Why a part does not fit in the schema.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SchemaError {
    /// Two parts define the same definition.
    #[error("the definition `{0}` is declared twice")]
    Defined(&'static str),
    /// An extension comes before its definition.
    #[error("an extension adds to `{0}`, but no part before it defines `{0}`")]
    Undefined(&'static str),
    /// A definition has a name two times.
    #[error("`{definition}#{name}` is declared twice")]
    Duplicate {
        /// The name of the definition.
        definition: &'static str,
        /// The name of the relation or the permission.
        name: &'static str,
    },
}

/// The full schema: one definition for each name, with the extensions in it.
#[derive(Clone, Debug, Default)]
pub struct Schema {
    /// The definitions, in registration order.
    definitions: Vec<Part>,
}

impl Schema {
    /// Returns `true` if the definition has this permission.
    pub fn has(&self, definition: &str, permission: &str) -> bool {
        self.definitions
            .iter()
            .filter(|part| part.definition == definition)
            .flat_map(|part| &part.permissions)
            .any(|(name, _)| *name == permission)
    }

    /// Adds a part. A definition comes one time, before its extensions, and a name comes one
    /// time in its definition. A part that does not fit changes nothing.
    pub fn add(&mut self, part: Part) -> Result<(), SchemaError> {
        // Find the definition of the part in the parts before it.
        let known = self
            .definitions
            .iter()
            .position(|definition| definition.definition == part.definition);
        match (known, part.extension) {
            // A definition comes one time, and an extension comes after its definition.
            (Some(_), false) => Err(SchemaError::Defined(part.definition)),
            (None, true) => Err(SchemaError::Undefined(part.definition)),

            // A new definition: each name comes one time.
            (None, false) => {
                unique(&[], &part)?;
                self.definitions.push(part);
                Ok(())
            }

            // An extension: its names are new in the definition, and go after the others.
            (Some(index), true) => {
                let definition = &mut self.definitions[index];
                unique(&definition.names().collect::<Vec<_>>(), &part)?;
                definition.relations.extend(part.relations);
                definition.permissions.extend(part.permissions);
                Ok(())
            }
        }
    }
}

/// Passes if each name of the part is new: not in `known`, and one time in the part.
fn unique(known: &[&'static str], part: &Part) -> Result<(), SchemaError> {
    let mut seen = known.to_vec();
    for name in part.names() {
        if seen.contains(&name) {
            return Err(SchemaError::Duplicate {
                definition: part.definition,
                name,
            });
        }
        seen.push(name);
    }
    Ok(())
}

/// The schema in the SpiceDB schema language.
impl fmt::Display for Schema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, definition) in self.definitions.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            writeln!(f, "definition {} {{", definition.definition)?;
            for (name, subject) in &definition.relations {
                writeln!(f, "    relation {name}: {subject}")?;
            }
            for (name, expression) in &definition.permissions {
                writeln!(f, "    permission {name} = {expression}")?;
            }
            writeln!(f, "}}")?;
        }
        Ok(())
    }
}
