use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::zip_probe::{extract_parsed_zip_entry, parse_zip_entries};

const DEX_HEADER_SIZE: usize = 0x70;
const NO_INDEX: u32 = u32::MAX;
const ACC_STATIC: u32 = 0x0008;
const ACC_NATIVE: u32 = 0x0100;
const ACC_INTERFACE: u32 = 0x0200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DexMemberKind {
    Method,
    Field,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedDexMember {
    pub(crate) declaring_class: String,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ApkDexIndex {
    classes: BTreeMap<String, DexClass>,
}

#[derive(Debug, Clone, Default)]
struct DexClass {
    access_flags: u32,
    superclass: Option<String>,
    interfaces: Vec<String>,
    methods: BTreeMap<MemberKey, DexMethod>,
    fields: BTreeMap<MemberKey, u32>,
}

#[derive(Debug, Clone, Copy, Default)]
struct DexMethod {
    access_flags: u32,
    is_direct: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct MemberKey {
    name: String,
    descriptor: String,
}

fn resolved_member(declaring_class: &str) -> ResolvedDexMember {
    ResolvedDexMember {
        declaring_class: declaring_class.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DexIndexError {
    Zip(String),
    Truncated(&'static str),
    Invalid(&'static str),
    InvalidIndex(&'static str, u32),
}

impl fmt::Display for DexIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zip(err) => write!(f, "APK ZIP: {err}"),
            Self::Truncated(what) => write!(f, "truncated DEX {what}"),
            Self::Invalid(what) => write!(f, "invalid DEX {what}"),
            Self::InvalidIndex(what, index) => {
                write!(f, "invalid DEX {what} index {index}")
            }
        }
    }
}

impl std::error::Error for DexIndexError {}

impl ApkDexIndex {
    pub(crate) fn from_apk(apk: &[u8]) -> Result<Self, DexIndexError> {
        let entries = parse_zip_entries(apk).map_err(|err| DexIndexError::Zip(err.to_string()))?;
        let mut dex_entries: Vec<_> = entries
            .iter()
            .filter(|entry| is_classes_dex(&entry.name))
            .collect();
        dex_entries.sort_by(|left, right| left.name.cmp(&right.name));

        let mut out = Self::default();
        for entry in dex_entries {
            let bytes = extract_parsed_zip_entry(apk, entry)
                .map_err(|err| DexIndexError::Zip(err.to_string()))?;
            out.add_dex(&bytes)?;
        }
        Ok(out)
    }

    pub(crate) fn contains_class(&self, descriptor: &str) -> bool {
        if self.classes.contains_key(descriptor) || platform_class_known(descriptor) {
            return true;
        }
        let Some(component) = descriptor.strip_prefix('[') else {
            return false;
        };
        matches!(component, "Z" | "B" | "C" | "S" | "I" | "J" | "F" | "D")
            || self.contains_class(component)
    }

    pub(crate) fn native_activity_class(&self) -> Option<&str> {
        self.classes
            .keys()
            .find(|descriptor| {
                self.is_assignable_to(descriptor, "Landroid/app/NativeActivity;")
                    && descriptor.as_str() != "Landroid/app/NativeActivity;"
            })
            .map(String::as_str)
    }

    #[cfg(test)]
    pub(crate) fn contains_member(
        &self,
        class_descriptor: &str,
        name: &str,
        descriptor: &str,
        kind: DexMemberKind,
        is_static: bool,
    ) -> bool {
        self.resolve_member(class_descriptor, name, descriptor, kind, is_static)
            .is_some()
    }

    pub(crate) fn resolve_member(
        &self,
        class_descriptor: &str,
        name: &str,
        descriptor: &str,
        kind: DexMemberKind,
        is_static: bool,
    ) -> Option<ResolvedDexMember> {
        let key = MemberKey {
            name: name.to_string(),
            descriptor: descriptor.to_string(),
        };
        match (kind, is_static) {
            (DexMemberKind::Method, false) => self.resolve_instance_method(class_descriptor, &key),
            (DexMemberKind::Method, true) => self.resolve_static_method(class_descriptor, &key),
            (DexMemberKind::Field, false) => self.resolve_instance_field(class_descriptor, &key),
            (DexMemberKind::Field, true) => self.resolve_static_field(class_descriptor, &key),
        }
    }

    fn resolve_instance_method(
        &self,
        class_descriptor: &str,
        key: &MemberKey,
    ) -> Option<ResolvedDexMember> {
        if self.is_interface(class_descriptor) {
            return self.resolve_interface_method(class_descriptor, key, &mut BTreeSet::new());
        }

        let mut current = Some(class_descriptor.to_string());
        let mut visited = BTreeSet::new();
        while let Some(class) = current {
            if !visited.insert(class.clone()) {
                return None;
            }
            if let Some(definition) = self.classes.get(&class) {
                if let Some(method) = definition.methods.get(key) {
                    if !method.is_direct {
                        return (method.access_flags & ACC_STATIC == 0)
                            .then(|| resolved_member(&class));
                    }
                }
                current = definition.superclass.clone();
            } else {
                if platform_member_staticness(&class, key, DexMemberKind::Method) == Some(false) {
                    return Some(resolved_member(&class));
                }
                current = platform_superclass(&class).map(str::to_string);
            }
        }

        self.classes
            .get(class_descriptor)
            .and_then(|class| class.methods.get(key))
            .filter(|method| method.is_direct && method.access_flags & ACC_STATIC == 0)
            .map(|_| resolved_member(class_descriptor))
    }

    fn resolve_interface_method(
        &self,
        interface: &str,
        key: &MemberKey,
        visited: &mut BTreeSet<String>,
    ) -> Option<ResolvedDexMember> {
        if !visited.insert(interface.to_string()) {
            return None;
        }
        if let Some(definition) = self.classes.get(interface) {
            if let Some(method) = definition.methods.get(key) {
                if !method.is_direct {
                    return (method.access_flags & ACC_STATIC == 0)
                        .then(|| resolved_member(interface));
                }
            }
            for parent in &definition.interfaces {
                if let Some(method) = self.resolve_interface_method(parent, key, visited) {
                    return Some(method);
                }
            }
        } else if platform_member_staticness(interface, key, DexMemberKind::Method) == Some(false) {
            return Some(resolved_member(interface));
        }
        None
    }

    fn resolve_static_method(
        &self,
        class_descriptor: &str,
        key: &MemberKey,
    ) -> Option<ResolvedDexMember> {
        let mut current = Some(class_descriptor.to_string());
        let mut visited = BTreeSet::new();
        while let Some(class) = current {
            if !visited.insert(class.clone()) {
                return None;
            }
            if let Some(definition) = self.classes.get(&class) {
                if let Some(method) = definition.methods.get(key) {
                    if method.is_direct {
                        return (method.access_flags & ACC_STATIC != 0)
                            .then(|| resolved_member(&class));
                    }
                }
                current = definition.superclass.clone();
            } else {
                if platform_member_staticness(&class, key, DexMemberKind::Method) == Some(true) {
                    return Some(resolved_member(&class));
                }
                current = platform_superclass(&class).map(str::to_string);
            }
        }
        None
    }

    fn resolve_instance_field(
        &self,
        class_descriptor: &str,
        key: &MemberKey,
    ) -> Option<ResolvedDexMember> {
        let mut current = Some(class_descriptor.to_string());
        let mut visited = BTreeSet::new();
        while let Some(class) = current {
            if !visited.insert(class.clone()) {
                return None;
            }
            if let Some(definition) = self.classes.get(&class) {
                if definition
                    .fields
                    .get(key)
                    .is_some_and(|flags| flags & ACC_STATIC == 0)
                {
                    return Some(resolved_member(&class));
                }
                current = definition.superclass.clone();
            } else {
                if platform_member_staticness(&class, key, DexMemberKind::Field) == Some(false) {
                    return Some(resolved_member(&class));
                }
                current = platform_superclass(&class).map(str::to_string);
            }
        }
        None
    }

    fn resolve_static_field(
        &self,
        class_descriptor: &str,
        key: &MemberKey,
    ) -> Option<ResolvedDexMember> {
        self.resolve_static_field_hierarchy(class_descriptor, key, &mut BTreeSet::new())
    }

    fn resolve_static_field_hierarchy(
        &self,
        class: &str,
        key: &MemberKey,
        visited: &mut BTreeSet<String>,
    ) -> Option<ResolvedDexMember> {
        if !visited.insert(class.to_string()) {
            return None;
        }
        if let Some(definition) = self.classes.get(class) {
            if definition
                .fields
                .get(key)
                .is_some_and(|flags| flags & ACC_STATIC != 0)
            {
                return Some(resolved_member(class));
            }
            for interface in &definition.interfaces {
                if let Some(field) = self.resolve_static_field_interface(interface, key, visited) {
                    return Some(field);
                }
            }
            if let Some(superclass) = &definition.superclass {
                return self.resolve_static_field_hierarchy(superclass, key, visited);
            }
        } else {
            if platform_member_staticness(class, key, DexMemberKind::Field) == Some(true) {
                return Some(resolved_member(class));
            }
            if let Some(superclass) = platform_superclass(class) {
                return self.resolve_static_field_hierarchy(superclass, key, visited);
            }
        }
        None
    }

    fn resolve_static_field_interface(
        &self,
        interface: &str,
        key: &MemberKey,
        visited: &mut BTreeSet<String>,
    ) -> Option<ResolvedDexMember> {
        if !visited.insert(interface.to_string()) {
            return None;
        }
        if let Some(definition) = self.classes.get(interface) {
            if definition
                .fields
                .get(key)
                .is_some_and(|flags| flags & ACC_STATIC != 0)
            {
                return Some(resolved_member(interface));
            }
            for parent in &definition.interfaces {
                if let Some(field) = self.resolve_static_field_interface(parent, key, visited) {
                    return Some(field);
                }
            }
        } else if platform_member_staticness(interface, key, DexMemberKind::Field) == Some(true) {
            return Some(resolved_member(interface));
        }
        None
    }

    fn is_interface(&self, descriptor: &str) -> bool {
        self.classes
            .get(descriptor)
            .is_some_and(|class| class.access_flags & ACC_INTERFACE != 0)
            || platform_is_interface(descriptor)
    }

    pub(crate) fn is_declared_native_method(
        &self,
        class_descriptor: &str,
        name: &str,
        descriptor: &str,
    ) -> bool {
        let key = MemberKey {
            name: name.to_string(),
            descriptor: descriptor.to_string(),
        };
        self.classes
            .get(class_descriptor)
            .and_then(|class| class.methods.get(&key))
            .is_some_and(|method| method.access_flags & ACC_NATIVE != 0)
    }

    pub(crate) fn is_assignable_to(&self, actual: &str, expected: &str) -> bool {
        if actual == expected {
            return true;
        }
        if actual.starts_with('[') {
            if matches!(
                expected,
                "Ljava/lang/Object;" | "Ljava/lang/Cloneable;" | "Ljava/io/Serializable;"
            ) {
                return true;
            }
            let (Some(actual_component), Some(expected_component)) =
                (actual.strip_prefix('['), expected.strip_prefix('['))
            else {
                return false;
            };
            let actual_primitive = is_primitive_descriptor(actual_component);
            let expected_primitive = is_primitive_descriptor(expected_component);
            return if actual_primitive || expected_primitive {
                actual_component == expected_component
            } else {
                self.is_assignable_to(actual_component, expected_component)
            };
        }

        let mut pending = vec![actual.to_string()];
        let mut visited = BTreeSet::new();
        while let Some(class) = pending.pop() {
            if !visited.insert(class.clone()) {
                continue;
            }
            if let Some(definition) = self.classes.get(&class) {
                if let Some(superclass) = &definition.superclass {
                    if superclass == expected {
                        return true;
                    }
                    pending.push(superclass.clone());
                }
                for interface in &definition.interfaces {
                    if interface == expected {
                        return true;
                    }
                    pending.push(interface.clone());
                }
            } else if let Some(superclass) = platform_superclass(&class) {
                if superclass == expected {
                    return true;
                }
                pending.push(superclass.to_string());
            }
        }
        false
    }

    fn add_dex(&mut self, bytes: &[u8]) -> Result<(), DexIndexError> {
        let dex = ParsedDex::parse(bytes)?;

        for class_def in dex.class_defs()? {
            let descriptor = dex.type_name(class_def.class_idx)?.to_string();
            let superclass = if class_def.superclass_idx == NO_INDEX {
                None
            } else {
                Some(dex.type_name(class_def.superclass_idx)?.to_string())
            };
            if self.classes.contains_key(&descriptor) {
                return Err(DexIndexError::Invalid("duplicate class definition"));
            }
            let mut class = DexClass::default();
            class.access_flags = class_def.access_flags;
            class.superclass = superclass;
            class.interfaces = dex.read_type_list(class_def.interfaces_off)?;
            if class_def.class_data_off != 0 {
                dex.read_class_data(class_def.class_data_off, &descriptor, &mut class)?;
            }
            self.classes.insert(descriptor, class);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn insert_test_class(&mut self, descriptor: &str, superclass: Option<&str>) {
        self.classes.insert(
            descriptor.to_string(),
            DexClass {
                access_flags: 0,
                superclass: superclass.map(str::to_string),
                ..DexClass::default()
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn insert_test_method(
        &mut self,
        class_descriptor: &str,
        name: &str,
        descriptor: &str,
        is_static: bool,
        is_native: bool,
    ) {
        let class = self
            .classes
            .get_mut(class_descriptor)
            .expect("test class must be inserted first");
        let mut access_flags = 0;
        if is_static {
            access_flags |= ACC_STATIC;
        }
        if is_native {
            access_flags |= ACC_NATIVE;
        }
        class.methods.insert(
            MemberKey {
                name: name.to_string(),
                descriptor: descriptor.to_string(),
            },
            DexMethod {
                access_flags,
                is_direct: is_static || name == "<init>",
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn mark_test_method_direct(
        &mut self,
        class_descriptor: &str,
        name: &str,
        descriptor: &str,
    ) {
        self.classes
            .get_mut(class_descriptor)
            .and_then(|class| {
                class.methods.get_mut(&MemberKey {
                    name: name.to_string(),
                    descriptor: descriptor.to_string(),
                })
            })
            .expect("test method must be inserted first")
            .is_direct = true;
    }

    #[cfg(test)]
    pub(crate) fn mark_test_class_as_interface(&mut self, descriptor: &str) {
        self.classes
            .get_mut(descriptor)
            .expect("test class must be inserted first")
            .access_flags |= ACC_INTERFACE;
    }

    #[cfg(test)]
    pub(crate) fn add_test_interface(&mut self, descriptor: &str, interface: &str) {
        self.classes
            .get_mut(descriptor)
            .expect("test class must be inserted first")
            .interfaces
            .push(interface.to_string());
    }

    #[cfg(test)]
    pub(crate) fn insert_test_field(
        &mut self,
        class_descriptor: &str,
        name: &str,
        descriptor: &str,
        is_static: bool,
    ) {
        let class = self
            .classes
            .get_mut(class_descriptor)
            .expect("test class must be inserted first");
        class.fields.insert(
            MemberKey {
                name: name.to_string(),
                descriptor: descriptor.to_string(),
            },
            if is_static { ACC_STATIC } else { 0 },
        );
    }
}

fn is_classes_dex(name: &str) -> bool {
    let Some(stem) = name.strip_prefix("classes") else {
        return false;
    };
    let Some(number) = stem.strip_suffix(".dex") else {
        return false;
    };
    number.is_empty() || number.bytes().all(|byte| byte.is_ascii_digit())
}

fn platform_class_known(descriptor: &str) -> bool {
    matches!(
        descriptor,
        "Landroid/app/Activity;"
            | "Landroid/app/NativeActivity;"
            | "Landroid/content/Context;"
            | "Landroid/content/ContextWrapper;"
            | "Landroid/content/Intent;"
            | "Landroid/os/Build;"
            | "Landroid/os/Build$VERSION;"
            | "Landroid/os/IBinder;"
            | "Landroid/os/Vibrator;"
            | "Landroid/view/ContextThemeWrapper;"
            | "Landroid/view/View;"
            | "Landroid/view/Window;"
            | "Landroid/view/inputmethod/InputMethodManager;"
            | "Ljava/io/File;"
            | "Ljava/io/Serializable;"
            | "Ljava/lang/ArrayIndexOutOfBoundsException;"
            | "Ljava/lang/Class;"
            | "Ljava/lang/ClassLoader;"
            | "Ljava/lang/ClassNotFoundException;"
            | "Ljava/lang/Cloneable;"
            | "Ljava/lang/NoClassDefFoundError;"
            | "Ljava/lang/NoSuchFieldError;"
            | "Ljava/lang/NoSuchMethodError;"
            | "Ljava/lang/NullPointerException;"
            | "Ljava/lang/Object;"
            | "Ljava/lang/OutOfMemoryError;"
            | "Ljava/lang/RuntimeException;"
            | "Ljava/lang/String;"
            | "Ljava/lang/Throwable;"
            | "Lorg/apache/http/Header;"
    )
}

fn platform_is_interface(descriptor: &str) -> bool {
    matches!(
        descriptor,
        "Ljava/io/Serializable;" | "Ljava/lang/Cloneable;" | "Lorg/apache/http/Header;"
    )
}

fn platform_superclass(descriptor: &str) -> Option<&'static str> {
    match descriptor {
        "Landroid/app/NativeActivity;" => Some("Landroid/app/Activity;"),
        "Landroid/app/Activity;" => Some("Landroid/view/ContextThemeWrapper;"),
        "Landroid/view/ContextThemeWrapper;" => Some("Landroid/content/ContextWrapper;"),
        "Landroid/content/ContextWrapper;" => Some("Landroid/content/Context;"),
        "Landroid/content/Context;"
        | "Landroid/content/Intent;"
        | "Landroid/os/Build;"
        | "Landroid/os/Build$VERSION;"
        | "Landroid/os/IBinder;"
        | "Landroid/os/Vibrator;"
        | "Landroid/view/Window;"
        | "Landroid/view/View;"
        | "Landroid/view/inputmethod/InputMethodManager;"
        | "Ljava/io/File;"
        | "Ljava/lang/Class;"
        | "Ljava/lang/ClassLoader;"
        | "Ljava/lang/String;"
        | "Lorg/apache/http/Header;"
        | "Ljava/lang/Object;" => Some("Ljava/lang/Object;"),
        "Ljava/lang/ArrayIndexOutOfBoundsException;"
        | "Ljava/lang/ClassNotFoundException;"
        | "Ljava/lang/NullPointerException;"
        | "Ljava/lang/OutOfMemoryError;"
        | "Ljava/lang/RuntimeException;" => Some("Ljava/lang/Throwable;"),
        "Ljava/lang/NoClassDefFoundError;"
        | "Ljava/lang/NoSuchFieldError;"
        | "Ljava/lang/NoSuchMethodError;" => Some("Ljava/lang/Throwable;"),
        "Ljava/lang/Throwable;" => Some("Ljava/lang/Object;"),
        _ => None,
    }
    .filter(|superclass| *superclass != descriptor)
}

fn platform_member_staticness(class: &str, key: &MemberKey, kind: DexMemberKind) -> Option<bool> {
    let member = (class, key.name.as_str(), key.descriptor.as_str(), kind);
    match member {
        (
            "Landroid/content/Context;",
            "INPUT_METHOD_SERVICE",
            "Ljava/lang/String;",
            DexMemberKind::Field,
        )
        | (
            "Landroid/os/Build;",
            "MANUFACTURER" | "MODEL",
            "Ljava/lang/String;",
            DexMemberKind::Field,
        )
        | ("Landroid/os/Build$VERSION;", "SDK_INT", "I", DexMemberKind::Field) => Some(true),
        (
            "Landroid/content/Context;",
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            DexMemberKind::Method,
        )
        | (
            "Landroid/content/Context;",
            "getApplicationContext",
            "()Landroid/content/Context;",
            DexMemberKind::Method,
        )
        | (
            "Landroid/content/Context;",
            "getPackageName",
            "()Ljava/lang/String;",
            DexMemberKind::Method,
        )
        | (
            "Landroid/content/Context;",
            "getClassLoader",
            "()Ljava/lang/ClassLoader;",
            DexMemberKind::Method,
        )
        | (
            "Landroid/content/Context;",
            "startActivity",
            "(Landroid/content/Intent;)V",
            DexMemberKind::Method,
        )
        | (
            "Landroid/content/ContextWrapper;",
            "getFilesDir" | "getCacheDir",
            "()Ljava/io/File;",
            DexMemberKind::Method,
        )
        | (
            "Landroid/app/Activity;",
            "getWindow",
            "()Landroid/view/Window;",
            DexMemberKind::Method,
        )
        | (
            "Landroid/view/Window;",
            "getDecorView",
            "()Landroid/view/View;",
            DexMemberKind::Method,
        )
        | (
            "Landroid/view/View;",
            "getWindowToken",
            "()Landroid/os/IBinder;",
            DexMemberKind::Method,
        )
        | (
            "Landroid/view/inputmethod/InputMethodManager;",
            "showSoftInput",
            "(Landroid/view/View;I)Z",
            DexMemberKind::Method,
        )
        | (
            "Landroid/view/inputmethod/InputMethodManager;",
            "hideSoftInputFromWindow",
            "(Landroid/os/IBinder;I)Z",
            DexMemberKind::Method,
        )
        | ("Landroid/os/Vibrator;", "vibrate", "(J)V", DexMemberKind::Method)
        | (
            "Ljava/io/File;",
            "getPath" | "getAbsolutePath",
            "()Ljava/lang/String;",
            DexMemberKind::Method,
        )
        | (
            "Ljava/lang/Class;",
            "getClassLoader",
            "()Ljava/lang/ClassLoader;",
            DexMemberKind::Method,
        )
        | (
            "Ljava/lang/ClassLoader;",
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            DexMemberKind::Method,
        )
        | (
            "Lorg/apache/http/Header;",
            "getName" | "getValue",
            "()Ljava/lang/String;",
            DexMemberKind::Method,
        ) => Some(false),
        _ => None,
    }
}

struct ParsedDex<'a> {
    bytes: &'a [u8],
    strings: Vec<String>,
    types: Vec<String>,
    protos: Vec<String>,
    field_ids: Vec<FieldId>,
    method_ids: Vec<MethodId>,
    class_defs_size: u32,
    class_defs_off: u32,
}

#[derive(Clone, Copy)]
struct FieldId {
    class_idx: u16,
    type_idx: u16,
    name_idx: u32,
}

#[derive(Clone, Copy)]
struct MethodId {
    class_idx: u16,
    proto_idx: u16,
    name_idx: u32,
}

#[derive(Clone, Copy)]
struct ClassDef {
    class_idx: u32,
    access_flags: u32,
    superclass_idx: u32,
    interfaces_off: u32,
    class_data_off: u32,
}

impl<'a> ParsedDex<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self, DexIndexError> {
        if bytes.len() < DEX_HEADER_SIZE {
            return Err(DexIndexError::Truncated("header"));
        }
        if &bytes[..4] != b"dex\n" || bytes[7] != 0 {
            return Err(DexIndexError::Invalid("magic"));
        }
        if read_u32(bytes, 0x20)? as usize != bytes.len() {
            return Err(DexIndexError::Invalid("file size"));
        }
        if read_u32(bytes, 0x24)? as usize != DEX_HEADER_SIZE {
            return Err(DexIndexError::Invalid("header size"));
        }
        if read_u32(bytes, 0x28)? != 0x1234_5678 {
            return Err(DexIndexError::Invalid("endian tag"));
        }

        let strings = read_strings(bytes)?;
        let types = read_types(bytes, &strings)?;
        let protos = read_protos(bytes, &types)?;
        let field_ids = read_field_ids(bytes)?;
        let method_ids = read_method_ids(bytes)?;
        Ok(Self {
            bytes,
            strings,
            types,
            protos,
            field_ids,
            method_ids,
            class_defs_size: read_u32(bytes, 0x60)?,
            class_defs_off: read_u32(bytes, 0x64)?,
        })
    }

    fn type_name(&self, index: u32) -> Result<&str, DexIndexError> {
        self.types
            .get(index as usize)
            .map(String::as_str)
            .ok_or(DexIndexError::InvalidIndex("type", index))
    }

    fn string(&self, index: u32) -> Result<&str, DexIndexError> {
        self.strings
            .get(index as usize)
            .map(String::as_str)
            .ok_or(DexIndexError::InvalidIndex("string", index))
    }

    fn class_defs(&self) -> Result<Vec<ClassDef>, DexIndexError> {
        let capacity = checked_table_len(
            self.class_defs_off,
            self.class_defs_size,
            32,
            self.bytes.len(),
        )?;
        let mut out = Vec::with_capacity(capacity);
        for index in 0..self.class_defs_size {
            let offset = table_offset(self.class_defs_off, index, 32, self.bytes.len())?;
            out.push(ClassDef {
                class_idx: read_u32(self.bytes, offset)?,
                access_flags: read_u32(self.bytes, offset + 4)?,
                superclass_idx: read_u32(self.bytes, offset + 8)?,
                interfaces_off: read_u32(self.bytes, offset + 12)?,
                class_data_off: read_u32(self.bytes, offset + 24)?,
            });
        }
        Ok(out)
    }

    fn read_type_list(&self, offset: u32) -> Result<Vec<String>, DexIndexError> {
        if offset == 0 {
            return Ok(Vec::new());
        }
        let count = read_u32(self.bytes, offset as usize)?;
        let items_offset = offset
            .checked_add(4)
            .ok_or(DexIndexError::Invalid("type-list offset overflow"))?;
        let capacity = checked_table_len(items_offset, count, 2, self.bytes.len())?;
        let mut out = Vec::with_capacity(capacity);
        for index in 0..count {
            let item = table_offset(items_offset, index, 2, self.bytes.len())?;
            out.push(
                self.type_name(u32::from(read_u16(self.bytes, item)?))?
                    .to_string(),
            );
        }
        Ok(out)
    }

    fn read_class_data(
        &self,
        class_data_off: u32,
        class_descriptor: &str,
        class: &mut DexClass,
    ) -> Result<(), DexIndexError> {
        let mut cursor = usize::try_from(class_data_off)
            .map_err(|_| DexIndexError::Invalid("class data offset"))?;
        let static_fields = read_uleb128(self.bytes, &mut cursor)?;
        let instance_fields = read_uleb128(self.bytes, &mut cursor)?;
        let direct_methods = read_uleb128(self.bytes, &mut cursor)?;
        let virtual_methods = read_uleb128(self.bytes, &mut cursor)?;

        self.read_encoded_fields(&mut cursor, static_fields, class_descriptor, class)?;
        self.read_encoded_fields(&mut cursor, instance_fields, class_descriptor, class)?;
        self.read_encoded_methods(&mut cursor, direct_methods, class_descriptor, class, true)?;
        self.read_encoded_methods(&mut cursor, virtual_methods, class_descriptor, class, false)?;
        Ok(())
    }

    fn read_encoded_fields(
        &self,
        cursor: &mut usize,
        count: u32,
        class_descriptor: &str,
        class: &mut DexClass,
    ) -> Result<(), DexIndexError> {
        let mut field_index = 0u32;
        for _ in 0..count {
            field_index = field_index
                .checked_add(read_uleb128(self.bytes, cursor)?)
                .ok_or(DexIndexError::Invalid("field index overflow"))?;
            let access_flags = read_uleb128(self.bytes, cursor)?;
            let field = self
                .field_ids
                .get(field_index as usize)
                .copied()
                .ok_or(DexIndexError::InvalidIndex("field", field_index))?;
            if self.type_name(u32::from(field.class_idx))? != class_descriptor {
                return Err(DexIndexError::Invalid("encoded field owner"));
            }
            if class
                .fields
                .insert(
                    MemberKey {
                        name: self.string(field.name_idx)?.to_string(),
                        descriptor: self.type_name(u32::from(field.type_idx))?.to_string(),
                    },
                    access_flags,
                )
                .is_some()
            {
                return Err(DexIndexError::Invalid("duplicate encoded field"));
            }
        }
        Ok(())
    }

    fn read_encoded_methods(
        &self,
        cursor: &mut usize,
        count: u32,
        class_descriptor: &str,
        class: &mut DexClass,
        is_direct: bool,
    ) -> Result<(), DexIndexError> {
        let mut method_index = 0u32;
        for _ in 0..count {
            method_index = method_index
                .checked_add(read_uleb128(self.bytes, cursor)?)
                .ok_or(DexIndexError::Invalid("method index overflow"))?;
            let access_flags = read_uleb128(self.bytes, cursor)?;
            let _code_off = read_uleb128(self.bytes, cursor)?;
            let method = self
                .method_ids
                .get(method_index as usize)
                .copied()
                .ok_or(DexIndexError::InvalidIndex("method", method_index))?;
            if self.type_name(u32::from(method.class_idx))? != class_descriptor {
                return Err(DexIndexError::Invalid("encoded method owner"));
            }
            let descriptor =
                self.protos
                    .get(method.proto_idx as usize)
                    .ok_or(DexIndexError::InvalidIndex(
                        "proto",
                        u32::from(method.proto_idx),
                    ))?;
            if class
                .methods
                .insert(
                    MemberKey {
                        name: self.string(method.name_idx)?.to_string(),
                        descriptor: descriptor.clone(),
                    },
                    DexMethod {
                        access_flags,
                        is_direct,
                    },
                )
                .is_some()
            {
                return Err(DexIndexError::Invalid("duplicate encoded method"));
            }
        }
        Ok(())
    }
}

fn is_primitive_descriptor(descriptor: &str) -> bool {
    matches!(descriptor, "Z" | "B" | "C" | "S" | "I" | "J" | "F" | "D")
}

fn read_strings(bytes: &[u8]) -> Result<Vec<String>, DexIndexError> {
    let size = read_u32(bytes, 0x38)?;
    let offset = read_u32(bytes, 0x3c)?;
    let mut out = Vec::with_capacity(checked_table_len(offset, size, 4, bytes.len())?);
    for index in 0..size {
        let id_offset = table_offset(offset, index, 4, bytes.len())?;
        let mut cursor = read_u32(bytes, id_offset)? as usize;
        let _utf16_len = read_uleb128(bytes, &mut cursor)?;
        let end = bytes[cursor..]
            .iter()
            .position(|byte| *byte == 0)
            .and_then(|len| cursor.checked_add(len))
            .ok_or(DexIndexError::Truncated("string data"))?;
        out.push(String::from_utf8_lossy(&bytes[cursor..end]).into_owned());
    }
    Ok(out)
}

fn read_types(bytes: &[u8], strings: &[String]) -> Result<Vec<String>, DexIndexError> {
    let size = read_u32(bytes, 0x40)?;
    let offset = read_u32(bytes, 0x44)?;
    let mut out = Vec::with_capacity(checked_table_len(offset, size, 4, bytes.len())?);
    for index in 0..size {
        let id_offset = table_offset(offset, index, 4, bytes.len())?;
        let string_index = read_u32(bytes, id_offset)?;
        out.push(
            strings
                .get(string_index as usize)
                .cloned()
                .ok_or(DexIndexError::InvalidIndex("string", string_index))?,
        );
    }
    Ok(out)
}

fn read_protos(bytes: &[u8], types: &[String]) -> Result<Vec<String>, DexIndexError> {
    let size = read_u32(bytes, 0x48)?;
    let offset = read_u32(bytes, 0x4c)?;
    let mut out = Vec::with_capacity(checked_table_len(offset, size, 12, bytes.len())?);
    for index in 0..size {
        let id_offset = table_offset(offset, index, 12, bytes.len())?;
        let return_type = read_u32(bytes, id_offset + 4)?;
        let parameters_off = read_u32(bytes, id_offset + 8)?;
        let mut descriptor = String::from("(");
        if parameters_off != 0 {
            let list_offset = parameters_off as usize;
            let parameter_count = read_u32(bytes, list_offset)?;
            for parameter in 0..parameter_count {
                let item = table_offset(parameters_off + 4, parameter, 2, bytes.len())?;
                let type_index = read_u16(bytes, item)?;
                descriptor.push_str(
                    types
                        .get(type_index as usize)
                        .ok_or(DexIndexError::InvalidIndex("type", u32::from(type_index)))?,
                );
            }
        }
        descriptor.push(')');
        descriptor.push_str(
            types
                .get(return_type as usize)
                .ok_or(DexIndexError::InvalidIndex("type", return_type))?,
        );
        out.push(descriptor);
    }
    Ok(out)
}

fn read_field_ids(bytes: &[u8]) -> Result<Vec<FieldId>, DexIndexError> {
    let size = read_u32(bytes, 0x50)?;
    let offset = read_u32(bytes, 0x54)?;
    let mut out = Vec::with_capacity(checked_table_len(offset, size, 8, bytes.len())?);
    for index in 0..size {
        let item = table_offset(offset, index, 8, bytes.len())?;
        out.push(FieldId {
            class_idx: read_u16(bytes, item)?,
            type_idx: read_u16(bytes, item + 2)?,
            name_idx: read_u32(bytes, item + 4)?,
        });
    }
    Ok(out)
}

fn read_method_ids(bytes: &[u8]) -> Result<Vec<MethodId>, DexIndexError> {
    let size = read_u32(bytes, 0x58)?;
    let offset = read_u32(bytes, 0x5c)?;
    let mut out = Vec::with_capacity(checked_table_len(offset, size, 8, bytes.len())?);
    for index in 0..size {
        let item = table_offset(offset, index, 8, bytes.len())?;
        out.push(MethodId {
            class_idx: read_u16(bytes, item)?,
            proto_idx: read_u16(bytes, item + 2)?,
            name_idx: read_u32(bytes, item + 4)?,
        });
    }
    Ok(out)
}

fn checked_table_len(
    base: u32,
    count: u32,
    item_size: u32,
    file_len: usize,
) -> Result<usize, DexIndexError> {
    let byte_len = count
        .checked_mul(item_size)
        .ok_or(DexIndexError::Invalid("table size overflow"))?;
    let end = base
        .checked_add(byte_len)
        .ok_or(DexIndexError::Invalid("table offset overflow"))?;
    if end as usize > file_len {
        return Err(DexIndexError::Truncated("table"));
    }
    usize::try_from(count).map_err(|_| DexIndexError::Invalid("table count"))
}

fn table_offset(base: u32, index: u32, item_size: u32, len: usize) -> Result<usize, DexIndexError> {
    let offset = base
        .checked_add(
            index
                .checked_mul(item_size)
                .ok_or(DexIndexError::Invalid("table offset overflow"))?,
        )
        .ok_or(DexIndexError::Invalid("table offset overflow"))? as usize;
    let end = offset
        .checked_add(item_size as usize)
        .ok_or(DexIndexError::Invalid("table offset overflow"))?;
    if end > len {
        return Err(DexIndexError::Truncated("table"));
    }
    Ok(offset)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, DexIndexError> {
    let raw = bytes
        .get(offset..offset + 2)
        .ok_or(DexIndexError::Truncated("u16"))?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DexIndexError> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or(DexIndexError::Truncated("u32"))?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn read_uleb128(bytes: &[u8], cursor: &mut usize) -> Result<u32, DexIndexError> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = *bytes
            .get(*cursor)
            .ok_or(DexIndexError::Truncated("ULEB128"))?;
        *cursor = (*cursor)
            .checked_add(1)
            .ok_or(DexIndexError::Invalid("ULEB128 offset overflow"))?;
        if shift == 28 && byte & 0xf0 != 0 {
            return Err(DexIndexError::Invalid("ULEB128 overflow"));
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(DexIndexError::Invalid("ULEB128"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn classes_dex_names_are_bounded() {
        assert!(is_classes_dex("classes.dex"));
        assert!(is_classes_dex("classes2.dex"));
        assert!(!is_classes_dex("assets/classes.dex"));
        assert!(!is_classes_dex("classesx.dex"));
        assert!(!is_classes_dex("classes.dex.bak"));
    }

    #[test]
    fn platform_members_preserve_kind_and_staticness() {
        let index = ApkDexIndex::default();
        assert!(index.contains_member(
            "Landroid/content/Context;",
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            DexMemberKind::Method,
            false,
        ));
        assert!(index.contains_member(
            "Landroid/os/Vibrator;",
            "vibrate",
            "(J)V",
            DexMemberKind::Method,
            false,
        ));
        assert!(index.contains_member(
            "Landroid/os/Build;",
            "MODEL",
            "Ljava/lang/String;",
            DexMemberKind::Field,
            true,
        ));
        assert!(!index.contains_member(
            "Landroid/os/Build;",
            "MODEL",
            "Ljava/lang/String;",
            DexMemberKind::Method,
            true,
        ));
        assert!(!index.contains_member(
            "Landroid/os/Build;",
            "MODEL",
            "Ljava/lang/String;",
            DexMemberKind::Field,
            false,
        ));
    }

    #[test]
    fn platform_classes_and_arrays_are_not_wildcards() {
        let index = ApkDexIndex::default();
        assert!(index.contains_class("Ljava/lang/String;"));
        assert!(index.contains_class("[[I"));
        assert!(index.contains_class("[Ljava/lang/String;"));
        assert!(!index.contains_class("Lmade/up/Anything;"));
        assert!(!index.contains_class("[Lmade/up/Anything;"));
        assert!(
            index.is_assignable_to("Landroid/app/NativeActivity;", "Landroid/content/Context;")
        );
        assert!(index.is_assignable_to("[Ljava/lang/String;", "[Ljava/lang/Object;"));
        assert!(index.is_assignable_to("[[I", "[Ljava/lang/Object;"));
        assert!(!index.is_assignable_to("[I", "[J"));
    }

    #[test]
    fn member_resolution_follows_dalvik_jni_lookup_shapes() {
        let mut index = ApkDexIndex::default();
        index.insert_test_class("Ltest/Base;", Some("Ljava/lang/Object;"));
        index.insert_test_class("Ltest/Child;", Some("Ltest/Base;"));
        index.insert_test_method("Ltest/Base;", "virtualMethod", "()V", false, false);
        index.insert_test_method("Ltest/Base;", "<init>", "()V", false, false);
        index.insert_test_method("Ltest/Child;", "<init>", "()V", false, false);

        let inherited = index
            .resolve_member(
                "Ltest/Child;",
                "virtualMethod",
                "()V",
                DexMemberKind::Method,
                false,
            )
            .unwrap();
        assert_eq!(inherited.declaring_class, "Ltest/Base;");
        let constructor = index
            .resolve_member(
                "Ltest/Child;",
                "<init>",
                "()V",
                DexMemberKind::Method,
                false,
            )
            .unwrap();
        assert_eq!(constructor.declaring_class, "Ltest/Child;");

        index.insert_test_method("Ltest/Base;", "staticMethod", "()V", true, false);
        index.insert_test_method("Ltest/Child;", "staticMethod", "()V", false, false);
        index.mark_test_method_direct("Ltest/Child;", "staticMethod", "()V");
        assert!(
            index
                .resolve_member(
                    "Ltest/Child;",
                    "staticMethod",
                    "()V",
                    DexMemberKind::Method,
                    true,
                )
                .is_none(),
            "Dalvik stops at a same-signature non-static direct method"
        );

        index.insert_test_class("Ltest/ParentInterface;", Some("Ljava/lang/Object;"));
        index.mark_test_class_as_interface("Ltest/ParentInterface;");
        index.insert_test_method(
            "Ltest/ParentInterface;",
            "interfaceMethod",
            "()I",
            false,
            false,
        );
        index.insert_test_field("Ltest/ParentInterface;", "CONSTANT", "I", true);
        index.insert_test_class("Ltest/ChildInterface;", Some("Ljava/lang/Object;"));
        index.mark_test_class_as_interface("Ltest/ChildInterface;");
        index.add_test_interface("Ltest/ChildInterface;", "Ltest/ParentInterface;");
        index.add_test_interface("Ltest/Child;", "Ltest/ChildInterface;");

        let interface_method = index
            .resolve_member(
                "Ltest/ChildInterface;",
                "interfaceMethod",
                "()I",
                DexMemberKind::Method,
                false,
            )
            .unwrap();
        assert_eq!(interface_method.declaring_class, "Ltest/ParentInterface;");
        let interface_field = index
            .resolve_member("Ltest/Child;", "CONSTANT", "I", DexMemberKind::Field, true)
            .unwrap();
        assert_eq!(interface_field.declaring_class, "Ltest/ParentInterface;");
    }

    #[test]
    fn indexes_local_mcpe_declarations_and_interfaces_when_present() {
        let apk = [
            Path::new("/home/john/tmp/hgfs-deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk"),
            Path::new("/mnt/hgfs/deb13/AndroidGames/MineCraftPE-a0.15.0.1.apk"),
        ]
        .into_iter()
        .find(|path| path.exists());
        let Some(apk) = apk else {
            return;
        };
        let bytes = std::fs::read(apk).unwrap();
        let index = ApkDexIndex::from_apk(&bytes).unwrap();

        assert_eq!(
            index.native_activity_class(),
            Some("Lcom/mojang/minecraftpe/MainActivity;")
        );
        assert!(index.contains_member(
            "Lcom/mojang/minecraftpe/MainActivity;",
            "getFileDataBytes",
            "(Ljava/lang/String;)[B",
            DexMemberKind::Method,
            false,
        ));
        assert!(index.is_declared_native_method(
            "Lcom/microsoft/xbox/idp/interop/Interop;",
            "ticket_callback",
            "(Ljava/lang/String;IILjava/lang/String;)V",
        ));
        assert!(index.is_assignable_to(
            "Lcom/mojang/minecraftpe/store/googleplay/GooglePlayStore;",
            "Lcom/mojang/minecraftpe/store/Store;",
        ));
    }
}
