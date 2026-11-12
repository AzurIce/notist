//! Split component discovery (pure, while the tree lives) from ordered host IO.
use super::*;
use notist_core::definitions::DefId;
use notist_html::{Component, components};

/// Component facts for an evaluated document, with no retained document tree.
pub struct HtmlBindings {
    document: DocumentHandle,
    ids: Vec<DefId>,
}

impl HtmlBindings {
    pub fn environment(&self) -> &Environment {
        &self.document.environment
    }
    /// Possible shells, not installed bindings. The nominal resource locator
    /// is never published; final binding still checks both entry candidates.
    pub fn candidates(&self) -> BTreeMap<DefId, Component> {
        let registry = self.document.environment.context().registry();
        self.ids
            .iter()
            .filter_map(|&id| {
                let definition = registry.definition(id)?;
                if definition.id.module == "notist" || definition.implementation.is_code() {
                    return None;
                }
                let signature = registry.signature(id).expect("resolved declaration");
                if !signature.may_return_content() {
                    return None;
                }
                let module = &self.document.environment.modules()[&definition.id.module_id()];
                Some((
                    id,
                    Component::candidate(
                        signature.clone(),
                        ModuleLocator::Resource(
                            module.path.parent().unwrap().join(
                                components::component_entries(&definition.id.name)[0].clone(),
                            ),
                        ),
                    ),
                ))
            })
            .collect()
    }
}

impl DocumentHandle {
    /// Project component identities without accessing the host or retaining IR.
    pub fn html_bindings(&self, root: &crate::Item) -> HtmlBindings {
        HtmlBindings {
            document: self.clone(),
            ids: components::component_definitions(root, self.environment.context().registry())
                .into_iter()
                .filter(|definition| {
                    definition.id.module != "notist"
                        && !definition.implementation.is_code()
                        && self
                            .environment
                            .context()
                            .registry()
                            .signature(definition.id.def_id())
                            .expect("resolved declaration")
                            .may_return_content()
                })
                .map(|d| d.id.def_id())
                .collect(),
        }
    }
}

impl<R: Resources> Vault<R> {
    /// Ordered resource selection and validation, independent of tree lifetime.
    pub fn resolve_html_bindings(
        &self,
        bindings: &HtmlBindings,
    ) -> Result<HtmlRegistry, VaultError> {
        self.check_handle(&bindings.document)?;
        bind_html_ids(
            bindings.environment(),
            &self.resources,
            &self.module_urls,
            bindings.ids.iter().copied(),
        )
        .map_err(VaultError::Environment)
    }
}
