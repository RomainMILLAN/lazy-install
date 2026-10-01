//! The set of applications, and its invariants.

use super::application::{AppId, AppName, Application, ScriptRef, Slug};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("an application named \"{0}\" already exists")]
    DuplicateName(String),
    #[error("\"{name}\" would share its log file ({slug}) with \"{other}\"")]
    DuplicateSlug {
        name: String,
        slug: String,
        other: String,
    },
    #[error("no such application")]
    NotFound,
}

/// The applications, in config order.
///
/// Immutable: every change returns a *candidate* catalog, and the session only
/// swaps it in once the config file has been written. A candidate that breaks an
/// invariant is never built, so it never reaches the disk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Catalog {
    apps: Vec<Application>,
    next_id: u32,
}

impl Catalog {
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builds a catalog from `(name, script)` pairs, in order, checking the
    /// invariants as if each had been added one by one.
    pub fn from_entries(
        entries: impl IntoIterator<Item = (AppName, ScriptRef)>,
    ) -> Result<Self, CatalogError> {
        let mut catalog = Catalog::empty();
        for (name, script) in entries {
            catalog = catalog.with_added(name, script)?.0;
        }
        Ok(catalog)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Application> {
        self.apps.iter()
    }

    pub fn len(&self) -> usize {
        self.apps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    pub fn get(&self, id: AppId) -> Option<&Application> {
        self.apps.iter().find(|a| a.id() == id)
    }

    pub fn ids(&self) -> Vec<AppId> {
        self.apps.iter().map(Application::id).collect()
    }

    pub fn with_added(
        &self,
        name: AppName,
        script: ScriptRef,
    ) -> Result<(Catalog, AppId), CatalogError> {
        self.check_unique(&name, None)?;
        let id = AppId::new(self.next_id);
        let mut next = self.clone();
        next.apps.push(Application::new(id, name, script));
        next.next_id += 1;
        Ok((next, id))
    }

    pub fn with_edited(
        &self,
        id: AppId,
        name: AppName,
        script: ScriptRef,
    ) -> Result<Catalog, CatalogError> {
        let pos = self.position(id)?;
        self.check_unique(&name, Some(id))?;
        let mut next = self.clone();
        next.apps[pos] = Application::new(id, name, script);
        Ok(next)
    }

    pub fn with_removed(&self, id: AppId) -> Result<Catalog, CatalogError> {
        let pos = self.position(id)?;
        let mut next = self.clone();
        next.apps.remove(pos);
        Ok(next)
    }

    fn position(&self, id: AppId) -> Result<usize, CatalogError> {
        self.apps
            .iter()
            .position(|a| a.id() == id)
            .ok_or(CatalogError::NotFound)
    }

    fn check_unique(&self, name: &AppName, except: Option<AppId>) -> Result<(), CatalogError> {
        let slug = Slug::from_name(name);
        for other in self.apps.iter().filter(|a| Some(a.id()) != except) {
            if other.name().same_as(name) {
                return Err(CatalogError::DuplicateName(name.as_str().to_string()));
            }
            if *other.slug() == slug {
                return Err(CatalogError::DuplicateSlug {
                    name: name.as_str().to_string(),
                    slug: slug.as_str().to_string(),
                    other: other.name().as_str().to_string(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn name(s: &str) -> AppName {
        AppName::parse(s).unwrap()
    }

    fn script(s: &str) -> ScriptRef {
        ScriptRef::new(s, PathBuf::from(format!("/x/{s}")))
    }

    #[test]
    fn duplicate_names_are_refused_without_case() {
        let (c, _) = Catalog::empty()
            .with_added(name("Kitty"), script("k.sh"))
            .unwrap();
        assert_eq!(
            c.with_added(name("kitty"), script("other.sh")).unwrap_err(),
            CatalogError::DuplicateName("kitty".into())
        );
        assert_eq!(c.len(), 1, "the original is untouched");
    }

    #[test]
    fn slug_collisions_are_refused() {
        let (c, _) = Catalog::empty()
            .with_added(name("Foo Bar"), script("a.sh"))
            .unwrap();
        let err = c.with_added(name("foo-bar"), script("b.sh")).unwrap_err();
        assert!(matches!(err, CatalogError::DuplicateSlug { .. }), "{err:?}");
    }

    #[test]
    fn edit_may_keep_its_own_name_and_remove_works() {
        let (c, id) = Catalog::empty()
            .with_added(name("kitty"), script("k.sh"))
            .unwrap();
        let c2 = c.with_edited(id, name("Kitty"), script("k2.sh")).unwrap();
        assert_eq!(c2.get(id).unwrap().script().raw(), "k2.sh");
        assert_eq!(c.get(id).unwrap().script().raw(), "k.sh");
        let c3 = c2.with_removed(id).unwrap();
        assert!(c3.is_empty());
        assert_eq!(c3.with_removed(id), Err(CatalogError::NotFound));
    }

    #[test]
    fn ids_are_never_reused() {
        let (c, a) = Catalog::empty().with_added(name("a"), script("a")).unwrap();
        let c = c.with_removed(a).unwrap();
        let (_, b) = c.with_added(name("b"), script("b")).unwrap();
        assert_ne!(a, b);
    }
}
