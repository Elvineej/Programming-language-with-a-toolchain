//! Modules, part 1: a program in more than one file (spec
//! `docs/superpowers/specs/2026-10-23-elya-modules-part-1-design.md`).
//!
//! `load` reads the root program and every module it imports (D1), checks the
//! imports (E0600-E0606), and merges the files into ONE `ast::Module` for the
//! passes after it (D5): an imported module `a/b`'s function `f` is renamed
//! `a.b.f`, its unshadowed references are renamed with it, and `b.f` in an
//! importer becomes a reference to `a.b.f`. Types, constructors, effects and
//! operations keep their names and must be visible through an import (D3).
//!
//! Part 2 (spec 2026-10-24): an import grants exactly its effect list
//! (`import a/b / {IO}`; none is `{}`), checked after inference by
//! [`check_grants`] against the effects at positive positions of the module's
//! `pub` functions' types (E0606); E0607 for a list naming an effect the file
//! cannot see, E0608 for a list on a standard module.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::span::{SourceMap, Span, Spanned};
use crate::Session;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// A whole program: the merged module and every file's text.
pub struct Program {
    pub module: Module,
    pub sources: SourceMap,
    /// Spans already reported by a module diagnostic (E0600, E0601, E0603): a
    /// later pass's diagnostic on the same span is a duplicate of it (the
    /// modules review's R3).
    pub refused: HashSet<Span>,
    /// Every user import and what it grants (modules part 2, spec 2026-10-24):
    /// checked after inference by [`check_grants`].
    pub grants: Vec<Grant>,
}

/// One import of a user module: its span, its effect list (empty when none is
/// written: D2), the module's path, and the merged names of its `pub`
/// functions, each with its source name.
pub struct Grant {
    pub span: Span,
    /// The name the importing file calls the module by (its alias, or the
    /// path's last segment).
    pub name: String,
    pub list: Vec<Spanned<String>>,
    pub target: String,
    pub exports: Vec<(String, String)>,
}

/// Reads a module path (`a/b`) to its file's name and text, or `None`.
pub type Reader<'a> = dyn FnMut(&str) -> Option<(String, String)> + 'a;

/// The builtin modules: `m.f` names a builtin there, and `elya/<m>` imports one.
const BUILTIN_MODULES: &[&str] = &["io", "int", "float", "string", "time", "op"];

/// The standard modules written in Elya (spec 2026-10-24-elya-std-replay D1):
/// `elya/<name>` is this source, loaded as if it were the file `elya/<name>.elya`.
const STD_SOURCES: &[(&str, &str)] = &[
    ("elya/replay", include_str!("std/replay.elya")),
    // Capabilities part 3 (spec 2026-10-24-elya-capabilities-part-3 D1, D2).
    ("elya/console", include_str!("std/console.elya")),
    ("elya/clock", include_str!("std/clock.elya")),
    ("elya/host", include_str!("std/host.elya")),
];

fn std_source(path: &str) -> Option<&'static str> {
    STD_SOURCES
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, t)| *t)
}

/// One file of the program.
struct Unit {
    /// `a/b`; the root's is its file's stem.
    path: String,
    module: Module,
    /// The names this file's imports bind, to the module paths they name.
    binds: HashMap<String, String>,
    /// The module paths this file imports directly.
    imports: HashSet<String>,
    /// Import names whose module failed to load (E0600, E0601): a reference
    /// through one is already reported.
    failed: HashSet<String>,
}

struct Loader<'r, 'a> {
    session: &'r Session,
    read: &'r mut Reader<'a>,
    sources: SourceMap,
    units: Vec<Unit>,
    /// Module path -> index in `units`, once loaded.
    loaded: HashMap<String, usize>,
    /// The paths being loaded, outermost first (for E0601).
    stack: Vec<String>,
    diags: Vec<Diagnostic>,
}

/// The root module's path: its file's stem (`dir/prog.elya` is `prog`).
fn root_path(name: &str) -> String {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    // Without the extension no import can name the root (an import path is never
    // empty); `prog` would otherwise be the module `prog.elya` (the review's R7).
    file.strip_suffix(".elya").unwrap_or("").to_string()
}

/// The merged name of module `path`'s function `f`: not an identifier, so no
/// source can write it or clash with it.
fn mangle(path: &str, f: &str) -> String {
    format!("{}.{f}", path.replace('/', "."))
}

impl<'r, 'a> Loader<'r, 'a> {
    /// Parses `text` as module `path` and loads its imports, depth first. Returns
    /// its index in `units`.
    fn unit(&mut self, path: String, module: Module) -> usize {
        self.stack.push(path.clone());
        let mut binds = HashMap::new();
        let mut imports = HashSet::new();
        let mut failed = HashSet::new();
        for imp in &module.imports {
            let span = imp.span;
            let target = imp.node.path.join("/");
            if imp.node.path.first().map(String::as_str) == Some("elya")
                && std_source(&target).is_none()
            {
                // Part 2 D5: builtins are in scope without an import, and the
                // root holds every capability.
                if let Some(effects) = &imp.node.effects {
                    let at = effects.first().map_or(span, |e| e.span);
                    self.diags.push(
                        Diagnostic::error("E0608", "a standard module takes no effect list")
                            .with_label(at, "an effect allow-list")
                            .with_help(format!(
                                "write `import {target}`: the program itself holds every capability"
                            )),
                    );
                }
                let std_ok = imp.node.path.len() == 2
                    && BUILTIN_MODULES.contains(&imp.node.path[1].as_str());
                if std_ok && imp.node.alias.is_some() {
                    let m = &imp.node.path[1];
                    self.diags.push(
                        Diagnostic::error(
                            "E0604",
                            format!("the standard module `{target}` keeps its name `{m}`"),
                        )
                        .with_label(span, "a builtin module cannot be renamed")
                        .with_help(format!("write `import {target}` and call `{m}.f`")),
                    );
                } else if !std_ok {
                    self.diags.push(
                        Diagnostic::error("E0600", format!("no module `{target}`"))
                            .with_label(span, "not in the standard library")
                            .with_help(format!(
                                "the standard modules are {}",
                                BUILTIN_MODULES
                                    .iter()
                                    .map(|m| format!("`elya/{m}`"))
                                    .chain(STD_SOURCES.iter().map(|(p, _)| format!("`{p}`")))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )),
                    );
                }
                continue;
            }
            let name = imp
                .node
                .alias
                .clone()
                .unwrap_or_else(|| imp.node.path.last().cloned().unwrap_or_default());
            if BUILTIN_MODULES.contains(&name.as_str()) || binds.contains_key(&name) {
                let what = if BUILTIN_MODULES.contains(&name.as_str()) {
                    "a builtin module"
                } else {
                    "another import"
                };
                self.diags.push(
                    Diagnostic::error(
                        "E0604",
                        format!("the import name `{name}` is already {what}"),
                    )
                    .with_label(span, "a second meaning for this name")
                    .with_help(format!("give it another name: `import {target} as ...`")),
                );
                continue;
            }
            // A cycle: the module is being loaded, or is the root itself.
            let in_stack = self.stack.iter().position(|p| *p == target);
            if let Some(i) = in_stack {
                let mut chain: Vec<String> = self.stack[i..].to_vec();
                chain.push(target.clone());
                self.diags.push(
                    Diagnostic::error(
                        "E0601",
                        format!(
                            "an import cycle: {}",
                            chain
                                .iter()
                                .map(|p| format!("`{p}`"))
                                .collect::<Vec<_>>()
                                .join(" -> ")
                        ),
                    )
                    .with_label(span, "closes the cycle"),
                );
                failed.insert(name);
                continue;
            }
            if !self.loaded.contains_key(&target) {
                let found = match std_source(&target) {
                    Some(text) => Some((format!("{target}.elya"), text.to_string())),
                    None => (self.read)(&target),
                };
                match found {
                    None => {
                        self.diags.push(
                            Diagnostic::error("E0600", format!("no module `{target}`"))
                                .with_label(span, "no such file")
                                .with_help(format!(
                                    "a module `{target}` is the file `{target}.elya` next to the program"
                                )),
                        );
                        failed.insert(name);
                        continue;
                    }
                    Some((file, text)) => {
                        let base = self.sources.add_file(file, text.clone());
                        let (m, pd) = crate::parse::parse_module_at(self.session, &text, base);
                        self.diags.extend(pd);
                        let i = self.unit(target.clone(), m);
                        self.loaded.insert(target.clone(), i);
                    }
                }
            }
            imports.insert(target.clone());
            binds.insert(name, target);
        }
        self.stack.pop();
        self.units.push(Unit {
            path,
            module,
            binds,
            imports,
            failed,
        });
        self.units.len() - 1
    }
}

/// Loads the root program and every module it imports, read by `read`, and
/// merges them (D1-D6). A program with no user import is the module
/// `parse_module` builds, unchanged.
pub fn load(
    session: &Session,
    root_name: &str,
    root_text: &str,
    read: &mut Reader,
) -> (Program, Vec<Diagnostic>) {
    let (root, diags) = crate::parse::parse_module(session, root_text);
    let mut loader = Loader {
        session,
        read,
        sources: SourceMap::new(root_name, root_text),
        units: Vec::new(),
        loaded: HashMap::new(),
        stack: Vec::new(),
        diags,
    };
    let rp = root_path(root_name);
    loader.unit(rp, root);
    let Loader {
        sources,
        units,
        mut diags,
        ..
    } = loader;
    // `units` is in post-order: every module after the ones it imports, the root
    // last.
    let mut refused = HashSet::new();
    let grants = grants(&units, &mut diags);
    let module = merge(units, &mut diags, &mut refused);
    (
        Program {
            module,
            sources,
            refused,
            grants,
        },
        diags,
    )
}

/// Part 2: every loaded user import with its list and the target's exports;
/// and E0607 for a listed name that is not `IO` or an effect visible in the
/// importing file (D4), or that is listed twice.
fn grants(units: &[Unit], diags: &mut Vec<Diagnostic>) -> Vec<Grant> {
    let index: HashMap<&str, usize> = units
        .iter()
        .enumerate()
        .map(|(i, u)| (u.path.as_str(), i))
        .collect();
    let mut effect_owner: HashMap<&str, usize> = HashMap::new();
    for (i, u) in units.iter().enumerate() {
        for d in &u.module.decls {
            if let Decl::Effect(e) = &d.node {
                effect_owner.entry(e.name.as_str()).or_insert(i);
            }
        }
    }
    let root = units.len() - 1;
    let mut out = Vec::new();
    for (me, u) in units.iter().enumerate() {
        for imp in &u.module.imports {
            let target = imp.node.path.join("/");
            if imp.node.path.first().map(String::as_str) == Some("elya")
                && std_source(&target).is_none()
            {
                continue;
            }
            let list = imp.node.effects.clone().unwrap_or_default();
            let mut seen = HashSet::new();
            for e in &list {
                if !seen.insert(e.node.clone()) {
                    diags.push(
                        Diagnostic::error("E0607", format!("`{}` is listed twice", e.node))
                            .with_label(e.span, "a second time"),
                    );
                    continue;
                }
                let visible = e.node == "IO"
                    || effect_owner
                        .get(e.node.as_str())
                        .is_some_and(|&o| o == me || u.imports.contains(&units[o].path));
                if !visible {
                    diags.push(
                        Diagnostic::error("E0607", format!("no effect `{}` here", e.node))
                            .with_label(e.span, "not `IO` or an effect this file can see")
                            .with_help(
                                "an effect is visible where it is declared and in the files that import its module",
                            ),
                    );
                }
            }
            // Only a module that loaded (a failed one is reported already).
            let Some(&t) = index.get(target.as_str()) else {
                continue;
            };
            if !u.imports.contains(&target) || t == root {
                continue;
            }
            let exports = units[t]
                .module
                .decls
                .iter()
                .filter_map(|d| match &d.node {
                    Decl::Fn(f) if f.is_pub => Some((f.name.clone(), mangle(&target, &f.name))),
                    _ => None,
                })
                .collect();
            out.push(Grant {
                span: imp.span,
                name: imp
                    .node
                    .alias
                    .clone()
                    .unwrap_or_else(|| imp.node.path.last().cloned().unwrap_or_default()),
                list,
                target,
                exports,
            });
        }
    }
    out
}

/// Part 2 D1-D3, after inference: each import's module may expose, positively
/// in its `pub` functions' types, only the effects its list grants; else E0606
/// per (import, effect).
pub fn check_grants(
    grants: &[Grant],
    module: &Module,
    fns: &HashMap<String, crate::types::Ty>,
) -> Vec<Diagnostic> {
    let types: HashMap<&str, &TypeDecl> = module
        .decls
        .iter()
        .filter_map(|d| match &d.node {
            Decl::Type(t) => Some((t.name.as_str(), t)),
            _ => None,
        })
        .collect();
    let effects: HashMap<&str, &EffectDecl> = module
        .decls
        .iter()
        .filter_map(|d| match &d.node {
            Decl::Effect(e) => Some((e.name.as_str(), e)),
            _ => None,
        })
        .collect();
    let mut diags = Vec::new();
    for g in grants {
        // effect -> the first `pub` function exposing it, in declaration order.
        let mut exposed: Vec<(String, String)> = Vec::new();
        for (name, merged) in &g.exports {
            let Some(t) = fns.get(merged) else {
                continue;
            };
            let mut walk = Interface {
                types: types.clone(),
                effects: effects.clone(),
                seen: HashSet::new(),
                out: Vec::new(),
            };
            walk.ty(t, true);
            for l in walk.out {
                if !exposed.iter().any(|(e, _)| *e == l) {
                    exposed.push((l, name.clone()));
                }
            }
        }
        for (effect, via) in exposed {
            if g.list.iter().any(|e| e.node == effect) {
                continue;
            }
            diags.push(
                Diagnostic::error(
                    "E0606",
                    format!(
                        "module `{}` can perform `{effect}`, which this import does not grant",
                        g.target
                    ),
                )
                .with_label(g.span, format!("grants no `{effect}`"))
                .with_help(format!(
                    "`{}.{via}` performs it, or hands it out; grant it with `/ {{{effect}}}` after the path if this module may",
                    g.name
                )),
            );
        }
    }
    diags
}

/// D1: collects the effect labels at positive positions of types, through
/// data's declared fields, effect labels' type arguments and effects'
/// operation signatures (the review's S1-S3: a closure the module makes can
/// leave through any of them).
struct Interface<'m> {
    types: HashMap<&'m str, &'m TypeDecl>,
    effects: HashMap<&'m str, &'m EffectDecl>,
    /// (declared type or effect, polarity) already walked.
    seen: HashSet<(String, bool)>,
    out: Vec<String>,
}

impl Interface<'_> {
    /// A function's own row and result keep `positive`, its parameters flip it;
    /// data keeps it, through its arguments and declared fields.
    fn ty(&mut self, t: &crate::types::Ty, positive: bool) {
        use crate::types::Ty;
        match t {
            Ty::Fn(ps, row, r) => {
                for (l, lab) in &row.labels {
                    self.label(l, positive);
                    // An effect's arguments flow both ways (`St(s)`: `put` and
                    // `get`).
                    for a in &lab.args {
                        self.ty(a, true);
                        self.ty(a, false);
                    }
                }
                for p in ps {
                    self.ty(p, !positive);
                }
                self.ty(r, positive);
            }
            Ty::Con(n, args) => {
                for a in args {
                    self.ty(a, positive);
                }
                self.declared(n, positive);
            }
            Ty::Tuple(args) => {
                for a in args {
                    self.ty(a, positive);
                }
            }
            _ => {}
        }
    }

    /// Label `l` in a row at `positive`: counted when positive (the module
    /// performs it). Either way its operations carry values: performing it, the
    /// module supplies the arguments and receives the answers; handling it (a
    /// callback's row, negative), it supplies the answers.
    fn label(&mut self, l: &str, positive: bool) {
        if positive && !self.out.iter().any(|x| x == l) {
            self.out.push(l.to_string());
        }
        let Some(e) = self.effects.get(l).copied() else {
            return;
        };
        if !self.seen.insert((format!("effect {l}"), positive)) {
            return;
        }
        for op in &e.ops {
            for p in &op.node.param_tys {
                self.ann(&p.node, positive);
            }
            self.ann(&op.node.ret.node, !positive);
        }
    }

    /// The labels written in data type `name`'s declared fields, at `positive`.
    fn declared(&mut self, name: &str, positive: bool) {
        let Some(t) = self.types.get(name).copied() else {
            return;
        };
        if !self.seen.insert((format!("type {name}"), positive)) {
            return;
        }
        for v in &t.variants {
            for f in &v.node.fields {
                self.ann(&f.node, positive);
            }
        }
    }

    fn ann(&mut self, a: &TypeAnn, positive: bool) {
        if a.name == "fn" {
            // Effects only: a row's variable (its lowercase tail) is not one.
            for l in crate::ast::row_tail(a.row.as_deref().unwrap_or(&[])).0 {
                self.label(&l.node, positive);
            }
            let n = a.args.len();
            for (i, x) in a.args.iter().enumerate() {
                let p = if i + 1 == n { positive } else { !positive };
                self.ann(&x.node, p);
            }
        } else {
            for x in &a.args {
                self.ann(&x.node, positive);
            }
            self.declared(&a.name, positive);
        }
    }
}

/// What every file declares: its functions (and whether each is `pub`), and the
/// module owning each type, constructor, effect and operation.
struct Decls {
    fns: Vec<HashMap<String, bool>>,
    /// (kind, name) -> the declaring module: the four kinds are separate name
    /// spaces, as in one file (the review's R1).
    owner: HashMap<(Kind, String), usize>,
    /// Each operation's effect (for E0205 in a module, the review's R2).
    op_effect: HashMap<String, String>,
    /// Each file's own type-level names: visible there, whichever file declared
    /// them first (spec 2026-10-30-elya-name-clashes D3).
    declared: Vec<HashSet<(Kind, String)>>,
}

/// The name spaces a type-level name can live in.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Type,
    Ctor,
    Effect,
    Op,
}

fn decls(units: &[Unit]) -> Decls {
    let mut fns = Vec::new();
    let mut owner = HashMap::new();
    let mut op_effect = HashMap::new();
    let mut declared = Vec::new();
    for (i, u) in units.iter().enumerate() {
        let mut f = HashMap::new();
        let mut own = HashSet::new();
        for d in &u.module.decls {
            match &d.node {
                Decl::Fn(fd) => {
                    f.insert(fd.name.clone(), fd.is_pub);
                }
                Decl::Type(t) => {
                    owner.entry((Kind::Type, t.name.clone())).or_insert(i);
                    own.insert((Kind::Type, t.name.clone()));
                    for v in &t.variants {
                        owner.entry((Kind::Ctor, v.node.name.clone())).or_insert(i);
                        own.insert((Kind::Ctor, v.node.name.clone()));
                    }
                }
                Decl::Effect(e) => {
                    owner.entry((Kind::Effect, e.name.clone())).or_insert(i);
                    own.insert((Kind::Effect, e.name.clone()));
                    for op in &e.ops {
                        owner.entry((Kind::Op, op.node.name.clone())).or_insert(i);
                        own.insert((Kind::Op, op.node.name.clone()));
                        op_effect
                            .entry(op.node.name.clone())
                            .or_insert_with(|| e.name.clone());
                    }
                }
            }
        }
        fns.push(f);
        declared.push(own);
    }
    Decls {
        fns,
        owner,
        op_effect,
        declared,
    }
}

fn merge(units: Vec<Unit>, diags: &mut Vec<Diagnostic>, refused: &mut HashSet<Span>) -> Module {
    let root = units.len() - 1;
    if units.len() == 1 && units[0].failed.is_empty() {
        // No user import: the module as parsed (only the checks of D2/D3 could
        // apply, and with one file there is nothing to check).
        return units.into_iter().next().map(|u| u.module).unwrap();
    }
    let info = decls(&units);
    // E0205 in a module (the review's R2): resolve sees its functions renamed
    // (`util.ask`), so it cannot tell that `ask` is an operation's name.
    for u in &units[..root] {
        for d in &u.module.decls {
            if let Decl::Fn(f) = &d.node {
                if let Some(effect) = info.op_effect.get(&f.name) {
                    diags.push(
                        Diagnostic::error(
                            "E0205",
                            format!(
                                "function `{}` has the name of an operation of `{effect}`",
                                f.name
                            ),
                        )
                        .with_label(d.span, "a call of this name performs the operation instead")
                        .with_help("rename the function; a local binding may shadow an operation, a top-level function may not"),
                    );
                }
            }
        }
    }
    let index: HashMap<&str, usize> = units
        .iter()
        .enumerate()
        .map(|(i, u)| (u.path.as_str(), i))
        .collect();
    let mut out = Vec::new();
    let mut imports = Vec::new();
    for (i, u) in units.iter().enumerate() {
        let mut rw = Rewriter {
            units: &units,
            index: &index,
            info: &info,
            me: i,
            is_root: i == root,
            scopes: Vec::new(),
            diags: Vec::new(),
            refused: Vec::new(),
        };
        for d in &u.module.decls {
            out.push(rw.decl(d));
        }
        diags.extend(rw.diags);
        refused.extend(rw.refused);
        if i == root {
            imports = u.module.imports.clone();
        }
    }
    Module {
        imports,
        decls: out,
    }
}

/// Rewrites one file's declarations: renames (D5), qualified references (D2), and
/// the visibility check (D3).
struct Rewriter<'u> {
    units: &'u [Unit],
    index: &'u HashMap<&'u str, usize>,
    info: &'u Decls,
    me: usize,
    is_root: bool,
    scopes: Vec<HashSet<String>>,
    diags: Vec<Diagnostic>,
    refused: Vec<Span>,
}

impl<'u> Rewriter<'u> {
    fn local(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(name))
    }

    fn bind(&mut self, name: &str) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string());
        }
    }

    /// This file's name for its own function `f`.
    fn own(&self, f: &str) -> String {
        if self.is_root {
            f.to_string()
        } else {
            mangle(&self.units[self.me].path, f)
        }
    }

    /// E0605 unless a type-level name is declared here or in a module this file
    /// imports. Names no module declares (builtins, unknowns) are not ours.
    fn visible(&mut self, kind: Kind, name: &str, span: Span) {
        let Some(&owner) = self.info.owner.get(&(kind, name.to_string())) else {
            return;
        };
        // D3: a name this file or a file it imports declares is visible here,
        // though another file declared it first (that clash is E0209's, or
        // E0202's for an operation; the review's F1: an imported file's own
        // declaration counted only when it was the first).
        let key = (kind, name.to_string());
        let declares = |j: usize| self.info.declared[j].contains(&key);
        if owner == self.me
            || declares(self.me)
            || self.units[self.me]
                .imports
                .iter()
                .any(|p| self.index.get(p.as_str()).is_some_and(|&j| declares(j)))
            || self.units[self.me]
                .imports
                .contains(&self.units[owner].path)
        {
            return;
        }
        let d = if owner == self.units.len() - 1 {
            // The root: no module can import it (E0601).
            Diagnostic::error(
                "E0605",
                format!("`{name}` is declared in the root program, which no module can import"),
            )
            .with_label(span, "not visible here")
            .with_help("move the declaration into a module that both files import")
        } else {
            let path = &self.units[owner].path;
            Diagnostic::error(
                "E0605",
                format!("`{name}` is declared in module `{path}`, which this file does not import"),
            )
            .with_label(span, "not visible here")
            .with_help(format!("add `import {path}`"))
        };
        self.diags.push(d);
    }

    fn decl(&mut self, d: &Spanned<Decl>) -> Spanned<Decl> {
        let node = match &d.node {
            Decl::Fn(f) => {
                self.scopes.push(HashSet::new());
                for p in &f.params {
                    self.bind(&p.node.name);
                    if let Some(a) = &p.node.ann {
                        self.ann(a);
                    }
                }
                if let Some(a) = &f.ret_ann {
                    self.ann(a);
                }
                if let Some(row) = &f.effect_row {
                    for e in crate::ast::row_tail(row).0 {
                        self.visible(Kind::Effect, &e.node, e.span);
                    }
                }
                let body = self.block(&f.body);
                self.scopes.pop();
                Decl::Fn(FnDecl {
                    name: self.own(&f.name),
                    body: Rc::new(body),
                    ..f.clone()
                })
            }
            Decl::Type(t) => {
                for v in &t.variants {
                    for fld in &v.node.fields {
                        self.ann(fld);
                    }
                }
                Decl::Type(t.clone())
            }
            Decl::Effect(e) => {
                for op in &e.ops {
                    for a in &op.node.param_tys {
                        self.ann(a);
                    }
                    for p in &op.node.params {
                        if let Some(a) = &p.node.ann {
                            self.ann(a);
                        }
                    }
                    self.ann(&op.node.ret);
                }
                Decl::Effect(e.clone())
            }
        };
        Spanned { node, span: d.span }
    }

    fn ann(&mut self, a: &Spanned<TypeAnn>) {
        self.visible(Kind::Type, &a.node.name, a.span);
        for x in &a.node.args {
            self.ann(x);
        }
        if let Some(row) = &a.node.row {
            for e in crate::ast::row_tail(row).0 {
                self.visible(Kind::Effect, &e.node, e.span);
            }
        }
    }

    fn block(&mut self, b: &Spanned<Block>) -> Spanned<Block> {
        self.scopes.push(HashSet::new());
        let mut stmts = Vec::new();
        for s in b.node.stmts.iter() {
            let node = match &s.node {
                Stmt::Let { name, ann, value } => {
                    if let Some(a) = ann {
                        self.ann(a);
                    }
                    let value = self.expr(value);
                    self.bind(name);
                    Stmt::Let {
                        name: name.clone(),
                        ann: ann.clone(),
                        value,
                    }
                }
                Stmt::Expr(e) => Stmt::Expr(self.expr(e)),
            };
            stmts.push(Spanned { node, span: s.span });
        }
        let tail = b.node.tail.as_ref().map(|t| Rc::new(self.expr(t)));
        self.scopes.pop();
        Spanned {
            node: Block {
                stmts: stmts.into(),
                tail,
            },
            span: b.span,
        }
    }

    fn pattern(&mut self, p: &Spanned<Pattern>) {
        match &p.node {
            Pattern::Var(n) => self.bind(n),
            Pattern::Ctor { name, args } => {
                self.visible(Kind::Ctor, name, p.span);
                for a in args {
                    self.pattern(a);
                }
            }
            Pattern::Wild | Pattern::Lit(_) => {}
        }
    }

    fn rc(&mut self, e: &Rc<Spanned<Expr>>) -> Rc<Spanned<Expr>> {
        Rc::new(self.expr(e))
    }

    fn expr(&mut self, e: &Spanned<Expr>) -> Spanned<Expr> {
        let span = e.span;
        let node = match &e.node {
            Expr::Var(name) => {
                if self.local(name) {
                    Expr::Var(name.clone())
                } else if self.info.fns[self.me].contains_key(name) {
                    Expr::Var(self.own(name))
                } else {
                    let root = self.units.len() - 1;
                    let kind = if name.starts_with(|c: char| c.is_ascii_uppercase()) {
                        Kind::Ctor
                    } else {
                        Kind::Op
                    };
                    // An operation first (spec 2026-10-30-elya-name-clashes
                    // D1): a root function of the same name is the root's error
                    // (E0205), and this file's `console_line(..)` still means its
                    // operation (or E0605, if this file cannot see it). Only a
                    // lowercase name can be both: a constructor is capitalised,
                    // a function is not.
                    let program_name = self.info.owner.contains_key(&(kind, name.clone()));
                    if !self.is_root && !program_name && self.info.fns[root].contains_key(name) {
                        // The root's functions keep their names in the merged
                        // module; they are not this file's (D2).
                        self.diags.push(
                            Diagnostic::error("E0200", format!("unresolved name `{name}`"))
                                .with_label(span, "not found in this scope"),
                        );
                    } else {
                        self.visible(kind, name, span);
                    }
                    Expr::Var(name.clone())
                }
            }
            Expr::Qualified { module, name } => {
                if self.units[self.me].failed.contains(module) {
                    // Its import is already reported (E0600, E0601).
                    self.refused.push(span);
                }
                match self.units[self.me].binds.get(module) {
                    None => e.node.clone(), // a builtin (or E0201 later)
                    Some(path) => {
                        let target = self.index[path.as_str()];
                        match self.info.fns[target].get(name) {
                            Some(true) => Expr::Var(mangle(path, name)),
                            Some(false) => {
                                self.diags.push(
                                    Diagnostic::error(
                                        "E0602",
                                        format!("`{name}` is private to module `{path}`"),
                                    )
                                    .with_label(span, "not `pub`")
                                    .with_help(format!(
                                        "mark it `pub fn {name}` in `{path}` to export it"
                                    )),
                                );
                                // The function exists: the rest checks as if
                                // exported, so this is the one diagnostic.
                                Expr::Var(mangle(path, name))
                            }
                            None => {
                                self.diags.push(
                                    Diagnostic::error(
                                        "E0603",
                                        format!("module `{path}` has no function `{name}`"),
                                    )
                                    .with_label(span, "not declared there"),
                                );
                                self.refused.push(span);
                                e.node.clone()
                            }
                        }
                    }
                }
            }
            Expr::Call { callee, args } => Expr::Call {
                callee: self.rc(callee),
                args: args.iter().map(|a| self.expr(a)).collect(),
            },
            Expr::Unary { op, expr } => Expr::Unary {
                op: *op,
                expr: self.rc(expr),
            },
            Expr::Binary { op, lhs, rhs } => Expr::Binary {
                op: *op,
                lhs: self.rc(lhs),
                rhs: self.rc(rhs),
            },
            Expr::If {
                cond,
                then_block,
                else_block,
            } => Expr::If {
                cond: self.rc(cond),
                then_block: Rc::new(self.block(then_block)),
                else_block: Rc::new(self.block(else_block)),
            },
            Expr::Block(b) => {
                let sb = self.block(&Spanned {
                    node: b.clone(),
                    span,
                });
                Expr::Block(sb.node)
            }
            Expr::Handle { body, handler } => {
                let body = self.rc(body);
                let mut clauses = Vec::new();
                for c in &handler.clauses {
                    if !handler.any {
                        if let Some(eff) = &c.node.effect {
                            self.visible(Kind::Effect, eff, c.span);
                        }
                        self.visible(Kind::Op, &c.node.op, c.span);
                    }
                    self.scopes.push(HashSet::new());
                    for p in &c.node.params {
                        self.bind(&p.node.name);
                        if let Some(a) = &p.node.ann {
                            self.ann(a);
                        }
                    }
                    let b = self.rc(&c.node.body);
                    self.scopes.pop();
                    clauses.push(Spanned {
                        node: OpClause {
                            body: b,
                            ..c.node.clone()
                        },
                        span: c.span,
                    });
                }
                let ret = handler.ret.as_ref().map(|r| {
                    self.scopes.push(HashSet::new());
                    self.bind(&r.binder);
                    let b = self.rc(&r.body);
                    self.scopes.pop();
                    ReturnClause {
                        binder: r.binder.clone(),
                        body: b,
                    }
                });
                Expr::Handle {
                    body,
                    handler: Rc::new(Handler {
                        multi: handler.multi,
                        any: handler.any,
                        clauses,
                        ret,
                    }),
                }
            }
            Expr::Resume { arg } => Expr::Resume { arg: self.rc(arg) },
            Expr::Match { scrutinee, arms } => {
                let scrutinee = self.rc(scrutinee);
                let arms = arms
                    .iter()
                    .map(|a| {
                        self.scopes.push(HashSet::new());
                        self.pattern(&a.node.pat);
                        let body = self.rc(&a.node.body);
                        self.scopes.pop();
                        Spanned {
                            node: MatchArm {
                                pat: a.node.pat.clone(),
                                body,
                            },
                            span: a.span,
                        }
                    })
                    .collect();
                Expr::Match { scrutinee, arms }
            }
            Expr::Lambda { params, body } => {
                self.scopes.push(HashSet::new());
                for p in params {
                    self.bind(&p.node.name);
                    if let Some(a) = &p.node.ann {
                        self.ann(a);
                    }
                }
                let body = self.block(body);
                self.scopes.pop();
                Expr::Lambda {
                    params: params.clone(),
                    body: Rc::new(body),
                }
            }
            Expr::Int(_)
            | Expr::Float(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Unit
            | Expr::Error => e.node.clone(),
        };
        Spanned { node, span }
    }
}
