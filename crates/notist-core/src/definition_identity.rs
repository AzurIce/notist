//! Reclaimable logical definition identities, independent of contract revisions.
//! A DefId is not evidence that two registries have equal implementations.
//! Owning references retain names; raw IDs never resolve through globals.
use crate::identity::{Id, IdAllocator};
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    hash::{Hash, Hasher},
    ops::Deref,
    sync::{Arc, LazyLock, Mutex, Weak},
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum DefinitionTag {}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ModuleTag {}

/// A logical declaration ID, distinct from module and source IDs even though
/// they share the same numeric machinery.
///
/// ```compile_fail
/// fn confuse(id: notist_core::source::SourceId) -> notist_core::definitions::DefId {
///     id
/// }
/// ```
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DefId(Id<DefinitionTag>);
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(Id<ModuleTag>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityExhausted;
impl fmt::Display for IdentityExhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("32-bit definition identity space exhausted")
    }
}
impl std::error::Error for IdentityExhausted {}
static NEXT_DEF: IdAllocator<DefinitionTag> = IdAllocator::new(1);
static NEXT_MODULE: IdAllocator<ModuleTag> = IdAllocator::new(1);
struct ModuleIdentity {
    id: ModuleId,
    name: String,
}
struct DefinitionIdentity {
    id: DefId,
    module: Arc<ModuleIdentity>,
    path: FunctionName,
}

/// An owned logical module name. ModuleId itself remains a four-byte value.
#[derive(Clone)]
pub struct ModuleRef(Arc<ModuleIdentity>);
impl PartialEq for ModuleRef {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}
impl Eq for ModuleRef {}
impl Deref for ModuleRef {
    type Target = str;
    fn deref(&self) -> &str {
        self.name()
    }
}
impl AsRef<str> for ModuleRef {
    fn as_ref(&self) -> &str {
        self.name()
    }
}
impl From<ModuleRef> for String {
    fn from(module: ModuleRef) -> Self {
        module.name().to_owned()
    }
}
impl PartialEq<str> for ModuleRef {
    fn eq(&self, other: &str) -> bool {
        self.name() == other
    }
}
impl PartialEq<&str> for ModuleRef {
    fn eq(&self, other: &&str) -> bool {
        self.name() == *other
    }
}
impl PartialEq<String> for ModuleRef {
    fn eq(&self, other: &String) -> bool {
        self.name() == other
    }
}
impl PartialEq<ModuleRef> for String {
    fn eq(&self, other: &ModuleRef) -> bool {
        self == other.name()
    }
}
impl fmt::Display for ModuleRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
#[cfg(feature = "serde")]
impl serde::Serialize for ModuleRef {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ModuleRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::try_new(<String as serde::Deserialize>::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)
    }
}
impl Hash for ModuleRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

/// Retained name ownership for resolved IR. This contains no implementation or
/// revision cache: those remain in the corresponding Registry/CheckedProgram.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Definitions {
    functions: Arc<BTreeMap<DefId, FunctionId>>,
    modules: Arc<BTreeMap<ModuleId, ModuleRef>>,
    module_names: Arc<BTreeMap<String, ModuleId>>,
}
impl Definitions {
    pub fn resolve(
        &mut self,
        instance: crate::definitions::FunctionInstance,
    ) -> crate::definitions::ResolvedInstance {
        crate::definitions::ResolvedInstance {
            function: self.insert(instance.function),
            arguments: instance.arguments,
        }
    }
    pub fn selector(
        &self,
        instance: &crate::definitions::ResolvedInstance,
    ) -> Result<crate::definitions::FunctionInstance, DefinitionError> {
        Ok(crate::definitions::FunctionInstance {
            function: self.function(instance.function)?.clone(),
            arguments: instance.arguments.clone(),
        })
    }
    pub fn function(&self, id: DefId) -> Result<&FunctionId, DefinitionError> {
        self.functions
            .get(&id)
            .ok_or(DefinitionError::MissingFunction(id))
    }
    pub fn module(&self, id: ModuleId) -> Result<&ModuleRef, DefinitionError> {
        self.modules
            .get(&id)
            .ok_or(DefinitionError::MissingModule(id))
    }
    pub fn module_id(&self, name: &str) -> Option<ModuleId> {
        self.module_names.get(name).copied()
    }
    pub fn find(&self, module: &str, name: &str) -> Option<DefId> {
        self.functions
            .values()
            .find(|f| f.module == module && f.name == name)
            .map(FunctionId::def_id)
    }
    pub fn insert_module(&mut self, module: ModuleRef) -> ModuleId {
        let id = module.id();
        if !self.modules.contains_key(&id) {
            Arc::make_mut(&mut self.module_names).insert(module.name().into(), id);
            Arc::make_mut(&mut self.modules).insert(id, module);
        }
        id
    }
    pub fn insert(&mut self, function: FunctionId) -> DefId {
        let id = function.def_id();
        self.insert_module(ModuleRef(function.0.module.clone()));
        if !self.functions.contains_key(&id) {
            Arc::make_mut(&mut self.functions).insert(id, function);
        }
        id
    }
    pub fn merge(&mut self, other: &Self) {
        if Arc::ptr_eq(&self.functions, &other.functions)
            && Arc::ptr_eq(&self.modules, &other.modules)
        {
            return;
        }
        if self.functions.is_empty() && self.modules.is_empty() {
            *self = other.clone();
            return;
        }
        for module in other.modules.values() {
            self.insert_module(module.clone());
        }
        for function in other.functions.values() {
            self.insert(function.clone());
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinitionError {
    MissingFunction(DefId),
    MissingModule(ModuleId),
}
impl fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingFunction(_) => {
                f.write_str("function identity is not owned by this definition snapshot")
            }
            Self::MissingModule(_) => {
                f.write_str("module identity is not owned by this definition snapshot")
            }
        }
    }
}
impl std::error::Error for DefinitionError {}
impl ModuleRef {
    pub fn try_new(name: impl AsRef<str>) -> Result<Self, IdentityExhausted> {
        let name = name.as_ref();
        let mut entries = IDENTITIES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(owner) = entries.modules.get(name).and_then(Weak::upgrade) {
            return Ok(Self(owner));
        }
        let owner = Arc::new(ModuleIdentity {
            id: ModuleId(NEXT_MODULE.allocate().ok_or(IdentityExhausted)?),
            name: name.into(),
        });
        entries.modules.insert(name.into(), Arc::downgrade(&owner));
        Ok(Self(owner))
    }
    pub fn new(name: impl AsRef<str>) -> Self {
        Self::try_new(name).expect("32-bit module identity space exhausted")
    }
    pub fn id(&self) -> ModuleId {
        self.0.id
    }
    pub fn name(&self) -> &str {
        &self.0.name
    }
}
impl fmt::Debug for ModuleRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ModuleRef").field(&self.name()).finish()
    }
}

/// Read-only spelling. To edit a name, construct a new owning reference instead
/// of changing strings independently of its numeric identity.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FunctionName {
    pub module: String,
    pub name: String,
}

#[derive(Default)]
struct Identities {
    modules: HashMap<String, Weak<ModuleIdentity>>,
    definitions: HashMap<ModuleId, HashMap<String, Weak<DefinitionIdentity>>>,
}
static IDENTITIES: LazyLock<Mutex<Identities>> = LazyLock::new(Default::default);
impl Drop for ModuleIdentity {
    fn drop(&mut self) {
        let mut entries = IDENTITIES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if entries
            .modules
            .get(&self.name)
            .is_some_and(|entry| std::ptr::eq(entry.as_ptr(), self))
        {
            entries.modules.remove(&self.name);
        }
    }
}
impl Drop for DefinitionIdentity {
    fn drop(&mut self) {
        let mut entries = IDENTITIES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(definitions) = entries.definitions.get_mut(&self.module.id) {
            if definitions
                .get(&self.path.name)
                .is_some_and(|entry| std::ptr::eq(entry.as_ptr(), self))
            {
                definitions.remove(&self.path.name);
            }
            if definitions.is_empty() {
                entries.definitions.remove(&self.module.id);
            }
        }
        // The lock is released before the module-owning field is dropped.
    }
}

/// Owning qualified-name reference, NOT a 32-bit ID. Clone shares both names.
/// Escaping content can retain this handle without retaining an entire Registry.
#[derive(Clone)]
pub struct FunctionId(Arc<DefinitionIdentity>);
impl FunctionId {
    pub fn try_new(
        module: impl AsRef<str>,
        name: impl AsRef<str>,
    ) -> Result<Self, IdentityExhausted> {
        let (module, name) = (module.as_ref(), name.as_ref());
        let mut entries = IDENTITIES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let owner = entries.modules.get(module).and_then(Weak::upgrade);
        if let Some(owner) = &owner
            && let Some(existing) = entries
                .definitions
                .get(&owner.id)
                .and_then(|definitions| definitions.get(name))
                .and_then(Weak::upgrade)
        {
            return Ok(Self(existing));
        }
        // A temporary module upgrade might be its last owner if another thread
        // just retired its last definition. Release the lock before error Drop.
        let ids = (|| {
            Ok::<_, IdentityExhausted>((
                DefId(NEXT_DEF.allocate().ok_or(IdentityExhausted)?),
                if let Some(owner) = &owner {
                    owner.id
                } else {
                    ModuleId(NEXT_MODULE.allocate().ok_or(IdentityExhausted)?)
                },
            ))
        })();
        let (id, module_id) = match ids {
            Ok(ids) => ids,
            Err(error) => {
                drop(entries);
                return Err(error);
            }
        };
        let owner = owner.unwrap_or_else(|| {
            let owner = Arc::new(ModuleIdentity {
                id: module_id,
                name: module.into(),
            });
            entries
                .modules
                .insert(module.into(), Arc::downgrade(&owner));
            owner
        });
        let identity = Arc::new(DefinitionIdentity {
            id,
            module: owner,
            path: FunctionName {
                module: module.into(),
                name: name.into(),
            },
        });
        entries
            .definitions
            .entry(module_id)
            .or_default()
            .insert(name.into(), Arc::downgrade(&identity));
        Ok(Self(identity))
    }
    pub fn new(module: impl AsRef<str>, name: impl AsRef<str>) -> Self {
        Self::try_new(module, name).expect("32-bit definition identity space exhausted")
    }
    pub fn def_id(&self) -> DefId {
        self.0.id
    }
    pub fn module_id(&self) -> ModuleId {
        self.0.module.id
    }
}
impl Deref for FunctionId {
    type Target = FunctionName;
    fn deref(&self) -> &FunctionName {
        &self.0.path
    }
}
impl PartialEq for FunctionId {
    fn eq(&self, other: &Self) -> bool {
        self.def_id() == other.def_id()
    }
}
impl Eq for FunctionId {}
impl PartialEq<FunctionId> for DefId {
    fn eq(&self, other: &FunctionId) -> bool {
        *self == other.def_id()
    }
}
impl PartialEq<DefId> for FunctionId {
    fn eq(&self, other: &DefId) -> bool {
        self.def_id() == *other
    }
}
impl Hash for FunctionId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.def_id().hash(state);
    }
}
impl PartialOrd for FunctionId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for FunctionId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Public component lists retain lexical rather than allocation order.
        self.0.path.cmp(&other.0.path)
    }
}
impl fmt::Debug for FunctionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FunctionId")
            .field("module", &self.module)
            .field("name", &self.name)
            .finish()
    }
}
impl fmt::Display for FunctionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.module, self.name)
    }
}
#[cfg(feature = "serde")]
impl serde::Serialize for FunctionId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(&self.0.path, serializer)
    }
}
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for FunctionId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = <FunctionName as serde::Deserialize>::deserialize(deserializer)?;
        Self::try_new(name.module, name.name).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_32_bit_and_owning_references_are_shared() {
        assert_eq!(size_of::<DefId>(), 4);
        assert_eq!(size_of::<ModuleId>(), 4);
        assert_eq!(size_of::<Option<DefId>>(), 4);
        assert_eq!(size_of::<Option<ModuleId>>(), 4);
        let a = FunctionId::new("identity-width", "first");
        let b = FunctionId::new("identity-width", "second");
        let duplicate = FunctionId::new("identity-width", "first");
        assert!(Arc::ptr_eq(&a.0, &duplicate.0));
        assert!(Arc::ptr_eq(&a.0, &a.clone().0));
        assert_eq!(a.module_id(), b.module_id());
        assert_ne!(a.def_id(), b.def_id());
        assert_ne!(a, FunctionId::new("other-identity-width", "first"));
    }
    #[test]
    fn last_owners_reclaim_keys_without_reusing_ids() {
        let a = FunctionId::new("identity-retirement", "first");
        let old = (a.def_id(), a.module_id());
        let weak = Arc::downgrade(&a.0);
        let other = FunctionId::new("identity-retirement", "other");
        drop(a);
        assert!(weak.upgrade().is_none());
        {
            let entries = IDENTITIES.lock().unwrap();
            assert!(!entries.definitions[&other.module_id()].contains_key("first"));
        }
        drop(other);
        {
            let entries = IDENTITIES.lock().unwrap();
            assert!(!entries.modules.contains_key("identity-retirement"));
            assert!(!entries.definitions.contains_key(&old.1));
        }
        let next = FunctionId::new("identity-retirement", "first");
        assert_ne!(old, (next.def_id(), next.module_id()));
    }
    #[test]
    fn allocation_exhaustion_is_explicit_and_never_wraps() {
        let next = IdAllocator::<DefinitionTag>::new(u32::MAX - 1);
        let allocate = || next.allocate().map(DefId).ok_or(IdentityExhausted);
        assert_eq!(format!("{:?}", allocate().unwrap()), "DefId(4294967294)");
        assert_eq!(allocate(), Err(IdentityExhausted));
        assert_eq!(allocate(), Err(IdentityExhausted));
    }
    #[test]
    fn concurrent_registration_uses_the_same_retained_identity() {
        let owner = FunctionId::new("identity-concurrent", "function");
        let workers: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(|| FunctionId::new("identity-concurrent", "function")))
            .collect();
        for worker in workers {
            assert_eq!(worker.join().unwrap(), owner);
        }
    }
    #[test]
    fn neither_id_has_a_16_bit_limit() {
        let names: Vec<_> = (0..70_000)
            .map(|i| FunctionId::new(format!("identity-wide-{i}"), "function"))
            .collect();
        assert_ne!(names[0].def_id(), names[69_999].def_id());
        assert_ne!(names[0].module_id(), names[69_999].module_id());
    }
}
