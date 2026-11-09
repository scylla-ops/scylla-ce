//! The catalog of the permissions: what a custom role can hold. A permission is an action on an
//! area, and it gives a list of route permissions.

mod instance;
#[cfg(test)]
mod pages;
mod project;
#[cfg(test)]
mod tests;
mod workspace;

use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

use crate::authz::{Permission, Resource, Schema};

/// The level of a role or of an area. A role holds the areas of its level and of the levels
/// under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// The whole instance.
    Platform,
    /// One workspace and its projects.
    Workspace,
    /// One project.
    Project,
}

impl Level {
    /// Returns the name of the level in the API.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Workspace => "workspace",
            Self::Project => "project",
        }
    }

    /// Reads the name of a level.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "platform" => Some(Self::Platform),
            "workspace" => Some(Self::Workspace),
            "project" => Some(Self::Project),
            _ => None,
        }
    }

    /// Returns `true` if a role of this level can hold an area of the other level.
    pub fn covers(self, other: Self) -> bool {
        match self {
            Self::Platform => true,
            Self::Workspace => other != Self::Platform,
            Self::Project => other == Self::Project,
        }
    }
}

/// A route permission on one definition.
#[derive(Clone, Copy, Debug)]
pub struct Route {
    /// The name of the definition.
    pub definition: &'static str,
    /// The name of the permission.
    pub permission: &'static str,
}

impl Route {
    /// Makes the route of a typed permission.
    const fn of<R: Resource>(permission: Permission<R>) -> Self {
        Self {
            definition: R::NAME,
            permission: permission.name(),
        }
    }
}

/// One permission of the catalog.
#[derive(Clone, Copy, Debug)]
pub struct CatalogPermission {
    /// The id: `<area>.<action>`.
    pub id: &'static str,
    /// The action: view, create, edit, delete, run or manage.
    pub action: &'static str,
    /// The ids of the permissions that this one needs.
    pub depends_on: &'static [&'static str],
    /// `true` if each role of the level of the area has it.
    pub implicit: bool,
    /// The route permissions that it gives. A permission with no route acts on the lists.
    pub routes: &'static [Route],
}

/// One area of the catalog.
#[derive(Clone, Copy, Debug)]
pub struct Area {
    /// The id of the area.
    pub id: &'static str,
    /// The lowest level where a role can hold the area.
    pub level: Level,
    /// `true` if only a role for apps can hold the area.
    pub machine: bool,
    /// The permissions of the area.
    pub permissions: &'static [CatalogPermission],
}

/// The areas, from the platform down to the project.
pub const AREAS: [&[Area]; 3] = [instance::AREAS, workspace::AREAS, project::AREAS];

/// The permission that gives the view of all the workspaces in a list.
pub const VIEW_ALL_WORKSPACES: &str = "workspaces.view";

/// The permission that gives the view of a project.
pub const VIEW_PROJECT: &str = "project.view";

/// Returns each area of the catalog.
pub fn areas() -> impl Iterator<Item = &'static Area> {
    AREAS.iter().flat_map(|group| group.iter())
}

/// Returns the permissions of the catalog that a role of a level can hold.
pub fn permissions_of(level: Level) -> impl Iterator<Item = &'static CatalogPermission> {
    areas()
        .filter(move |area| level.covers(area.level))
        .flat_map(|area| area.permissions)
}

/// Returns the ids of the permissions of a level that a person can hold.
pub fn of_people(level: Level) -> impl Iterator<Item = &'static str> {
    areas()
        .filter(move |area| !area.machine && level.covers(area.level))
        .flat_map(|area| area.permissions)
        .map(|permission| permission.id)
}

/// Returns a permission of the catalog and its area, by id.
pub fn find(id: &str) -> Option<(&'static Area, &'static CatalogPermission)> {
    areas().find_map(|area| {
        area.permissions
            .iter()
            .find(|permission| permission.id == id)
            .map(|permission| (area, permission))
    })
}

/// The ids of the permissions that give each route: by definition, then by route permission.
type Covering = HashMap<&'static str, HashMap<&'static str, Vec<&'static str>>>;

/// The permissions that give each route.
static COVERING: LazyLock<Covering> = LazyLock::new(|| {
    let mut map = Covering::new();
    for permission in areas().flat_map(|area| area.permissions) {
        for route in permission.routes {
            map.entry(route.definition)
                .or_default()
                .entry(route.permission)
                .or_default()
                .push(permission.id);
        }
    }
    map
});

/// Returns the ids of the permissions of the catalog that give a route permission on a
/// definition.
pub fn covering(definition: &str, permission: &str) -> &'static [&'static str] {
    COVERING
        .get(definition)
        .and_then(|routes| routes.get(permission))
        .map_or(&[], Vec::as_slice)
}

/// Returns the ids of the permissions that a relation of the schema gives on a definition, for a
/// role of a level. A permission needs each of its routes that the definition has, and one or more.
pub fn given_by(
    schema: &Schema,
    definition: &str,
    relation: &str,
    level: Level,
) -> Vec<&'static str> {
    permissions_of(level)
        .filter(|permission| {
            let mut routes = permission
                .routes
                .iter()
                .filter(|route| schema.has(definition, route.permission))
                .peekable();
            routes.peek().is_some()
                && routes.all(|route| schema.gives(definition, route.permission, relation))
        })
        .map(|permission| permission.id)
        .collect()
}

/// Why the permissions of a role are not valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PermissionError<'a> {
    /// The id is not in the catalog.
    #[error("{0} is not in the catalog")]
    Unknown(&'a str),
    /// The permission is above the level of the role.
    #[error("{0} is above the level of the role")]
    AboveLevel(&'a str),
    /// Only a role for apps can hold the permission, or a role for apps cannot.
    #[error("{0} does not fit the holders of the role")]
    Holder(&'a str),
}

/// Returns the ids of the permissions that a permission needs, with the permission itself: what
/// it depends on, then what these depend on.
pub fn needs(id: &'static str) -> BTreeSet<&'static str> {
    let mut found = BTreeSet::new();
    let mut next = vec![id];
    while let Some(id) = next.pop() {
        if found.insert(id) {
            next.extend(find(id).map_or(&[][..], |(_, permission)| permission.depends_on));
        }
    }
    found
}

/// Checks the permissions of a role, then adds the permissions that they depend on and the
/// implicit permissions of its level. Returns the ids in the order of the catalog.
pub fn complete<'a>(
    level: Level,
    for_apps: bool,
    ids: &'a [String],
) -> Result<Vec<&'static str>, PermissionError<'a>> {
    // Check each id, and keep it with what it needs.
    let mut kept = BTreeSet::new();
    for id in ids {
        let (area, permission) = find(id).ok_or(PermissionError::Unknown(id))?;
        if !level.covers(area.level) {
            return Err(PermissionError::AboveLevel(id));
        }
        if area.machine && !for_apps {
            return Err(PermissionError::Holder(id));
        }
        kept.extend(needs(permission.id));
    }

    // Add the implicit permissions, then read the catalog in its order.
    let chosen = areas()
        .filter(|area| level.covers(area.level))
        .flat_map(|area| area.permissions.iter().map(move |cell| (area, cell)))
        .filter(|(area, cell)| kept.contains(cell.id) || (cell.implicit && area.level == level))
        .map(|(_, cell)| cell.id)
        .collect();
    Ok(chosen)
}
