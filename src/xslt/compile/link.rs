//! The passes after all modules are loaded: import precedence (libxslt's
//! module numbering), merging attribute sets, and ordering global variables
//! by the globals they reference.

use super::*;

/// Variable names referenced by a value, including inside bodies.
fn refs_in_value(v: &VarValue, out: &mut Vec<Name>) {
    match v {
        VarValue::Select(e) => refs_in_expr(e, out),
        VarValue::Body(b) => refs_in_body(b, out),
        VarValue::Empty => {}
    }
}

fn refs_in_avt(a: &Avt, out: &mut Vec<Name>) {
    if let Avt::Parts(parts) = a {
        for p in parts {
            if let AvtPart::Expr(e) = p {
                refs_in_expr(e, out);
            }
        }
    }
}

fn refs_in_body(body: &Body, out: &mut Vec<Name>) {
    for i in body {
        match i {
            Instr::Text(_)
            | Instr::UnescapedText(_)
            | Instr::ApplyImports { .. }
            | Instr::Unsupported { .. } => {}
            Instr::Document {
                href,
                settings,
                body,
                ..
            } => {
                href.iter().for_each(|a| refs_in_avt(a, out));
                settings.iter().for_each(|(_, a)| refs_in_avt(a, out));
                refs_in_body(body, out);
            }
            Instr::LiteralElement {
                attributes, body, ..
            } => {
                attributes.iter().for_each(|(_, a)| refs_in_avt(a, out));
                refs_in_body(body, out);
            }
            Instr::ValueOf { select, .. } | Instr::CopyOf { select, .. } => {
                refs_in_expr(select, out)
            }
            Instr::Number { value, format, .. } => {
                if let Some(v) = value {
                    refs_in_expr(v, out);
                }
                refs_in_avt(format, out);
            }
            Instr::ApplyTemplates {
                select,
                sorts,
                params,
                ..
            } => {
                if let Some(s) = select {
                    refs_in_expr(s, out);
                }
                sorts.iter().for_each(|s| refs_in_expr(&s.select, out));
                params.iter().for_each(|p| refs_in_value(&p.value, out));
            }
            Instr::CallTemplate { params, .. } => {
                params.iter().for_each(|p| refs_in_value(&p.value, out))
            }
            Instr::ForEach {
                select,
                sorts,
                body,
                ..
            } => {
                refs_in_expr(select, out);
                sorts.iter().for_each(|s| refs_in_expr(&s.select, out));
                refs_in_body(body, out);
            }
            Instr::If { test, body, .. } => {
                refs_in_expr(test, out);
                refs_in_body(body, out);
            }
            Instr::Choose { whens, otherwise } => {
                for (t, b, _) in whens {
                    refs_in_expr(t, out);
                    refs_in_body(b, out);
                }
                if let Some(b) = otherwise {
                    refs_in_body(b, out);
                }
            }
            Instr::Variable(v) => refs_in_value(&v.value, out),
            Instr::Copy { body, .. } | Instr::Comment(body) | Instr::Message { body, .. } => {
                refs_in_body(body, out)
            }
            Instr::Element {
                name,
                namespace,
                body,
                ..
            }
            | Instr::Attribute {
                name,
                namespace,
                body,
                ..
            } => {
                refs_in_avt(name, out);
                if let Some(n) = namespace {
                    refs_in_avt(n, out);
                }
                refs_in_body(body, out);
            }
            Instr::ProcessingInstruction { name, body, .. } => {
                refs_in_avt(name, out);
                refs_in_body(body, out);
            }
        }
    }
}

fn refs_in_expr(e: &Expr, out: &mut Vec<Name>) {
    use crate::xpath::ast::PathStart;
    match e {
        Expr::Variable(n) => out.push(*n),
        Expr::Or(a, b)
        | Expr::And(a, b)
        | Expr::Compare(_, a, b)
        | Expr::Arith(_, a, b)
        | Expr::Union(a, b) => {
            refs_in_expr(a, out);
            refs_in_expr(b, out);
        }
        Expr::Negate(a) => refs_in_expr(a, out),
        Expr::Call(_, args) => args.iter().for_each(|a| refs_in_expr(a, out)),
        Expr::Filter(p, preds) => {
            refs_in_expr(p, out);
            preds.iter().for_each(|a| refs_in_expr(a, out));
        }
        Expr::Path(p) => {
            if let PathStart::Expr(s) = &p.start {
                refs_in_expr(s, out);
            }
            for s in &p.steps {
                s.predicates.iter().for_each(|a| refs_in_expr(a, out));
            }
        }
        Expr::Literal(_) | Expr::Number(_) => {}
    }
}

impl Compiler<'_> {
    /// Numbers the modules by a post-order walk of the import tree (the
    /// main module highest) and resolves what precedence decides: named
    /// templates and global variables keep only their highest-precedence
    /// definition (named templates the later one on a tie, globals the
    /// first).
    pub(super) fn assign_precedence(&mut self) {
        fn walk(modules: &[Vec<usize>], m: usize, rank: &mut [usize], next: &mut usize) {
            for &i in &modules[m] {
                walk(modules, i, rank, next);
            }
            rank[m] = *next;
            *next += 1;
        }
        let mut rank = vec![0; self.modules.len()];
        walk(&self.modules, 0, &mut rank, &mut 0);
        for t in &mut self.ss.templates {
            t.precedence = rank[t.precedence];
        }
        for rules in self.ss.rules.values_mut() {
            for r in rules {
                r.precedence = rank[r.precedence];
            }
        }
        for r in &mut self.ss.space {
            r.precedence = rank[r.precedence];
        }
        let mut key_order = Vec::new();
        for (&name, defs) in &mut self.ss.keys {
            for (i, k) in defs.iter_mut().enumerate() {
                k.precedence = rank[k.precedence];
                key_order.push((std::cmp::Reverse(k.precedence), k.order, name, i));
            }
        }
        key_order.sort_by_key(|&(p, o, _, _)| (p, o));
        self.ss.key_order = key_order.into_iter().map(|(_, _, n, i)| (n, i)).collect();
        for (index, t) in self.ss.templates.iter().enumerate() {
            if let Some(name) = t.name {
                match self.ss.named.get(&name) {
                    Some(&old) if self.ss.templates[old].precedence > t.precedence => {}
                    _ => {
                        self.ss.named.insert(name, index);
                    }
                }
            }
        }
        let mut best: HashMap<Name, usize> = HashMap::new();
        for (g, &m) in self.ss.globals.iter().zip(&self.global_modules) {
            let p = best.entry(g.name).or_insert(rank[m]);
            *p = (*p).max(rank[m]);
        }
        // On a tie (a stylesheet and its includes) the first declaration in
        // document order wins: libxslt appends globals to the stylesheet's
        // list and xsltEvalGlobalVariables skips names already defined.
        let mut keep = self.global_modules.iter().map(|&m| rank[m]);
        let mut kept = HashSet::new();
        self.ss
            .globals
            .retain(|g| keep.next() == Some(best[&g.name]) && kept.insert(g.name));

        // libxslt takes cdata-section-elements from the first module, in
        // precedence order, that has any (XSLT_GET_IMPORT_PTR); they don't
        // merge across imports.
        if let Some(top) = self.cdata_modules.iter().map(|&(m, _)| rank[m]).max() {
            self.ss.output.cdata_section_elements = self
                .cdata_modules
                .iter()
                .filter(|&&(m, _)| rank[m] == top)
                .map(|&(_, n)| n)
                .collect();
        }

        // Attribute sets merge as libxslt's xsltMergeSetsCB does: the
        // highest-precedence definition's attributes, then each lower one's
        // whose names aren't already there.
        for (name, mut defs) in std::mem::take(&mut self.module_sets) {
            defs.sort_by_key(|(m, _)| std::cmp::Reverse(rank[*m]));
            let mut defs = defs.into_iter().map(|(_, set)| set);
            let mut merged = defs.next().expect("a set has a definition");
            for lower in defs {
                merged.uses.extend(lower.uses);
                for attr in lower.attributes {
                    let key = self.attribute_key(&attr);
                    if !merged
                        .attributes
                        .iter()
                        .any(|a| self.attribute_key(a) == key)
                    {
                        merged.attributes.push(attr);
                    }
                }
            }
            self.ss.attribute_sets.insert(name, merged);
        }
    }

    /// libxslt's xsltResolveStylesheetAttributeSet: each set's list becomes
    /// its own attributes (from every definition with its name, in order),
    /// followed by those of the sets it uses, in turn, minus any whose
    /// compile-time name is already in the list. The set's own attributes
    /// therefore win and keep their place. A cycle is an error.
    pub(super) fn resolve_attribute_sets(&mut self) {
        let names: Vec<Name> = self.ss.attribute_sets.keys().copied().collect();
        let mut state: HashMap<Name, u8> = HashMap::new(); // 1 = resolving, 2 = done
        for n in names {
            self.resolve_set(n, &mut state, 0);
        }
    }

    fn resolve_set(&mut self, name: Name, state: &mut HashMap<Name, u8>, depth: usize) {
        match state.get(&name) {
            Some(2) => return,
            Some(_) => {
                self.errors.push(CompileError {
                    message: format!(
                        "xsl:attribute-set : use-attribute-sets recursion detected on {}",
                        self.names.str(name.1)
                    ),
                    location: Location::default(),
                });
                state.insert(name, 2);
                return;
            }
            None => {}
        }
        if depth > 100 {
            return;
        }
        state.insert(name, 1);
        let uses = self
            .ss
            .attribute_sets
            .get(&name)
            .map(|s| s.uses.clone())
            .unwrap_or_default();
        for used in uses {
            // An unknown set in use-attribute-sets is skipped, as in libxslt.
            if !self.ss.attribute_sets.contains_key(&used) {
                continue;
            }
            self.resolve_set(used, state, depth + 1);
            let other = self.ss.attribute_sets[&used].attributes.clone();
            for attr in other {
                let key = self.attribute_key(&attr);
                let present = self.ss.attribute_sets[&name]
                    .attributes
                    .iter()
                    .any(|a| self.attribute_key(a) == key);
                if !present {
                    self.ss
                        .attribute_sets
                        .get_mut(&name)
                        .expect("set exists")
                        .attributes
                        .push(attr);
                }
            }
        }
        if let Some(set) = self.ss.attribute_sets.get_mut(&name) {
            set.uses.clear();
        }
        state.insert(name, 2);
    }

    /// The compile-time name of an `xsl:attribute`, as libxslt's merge
    /// compares it: local name and namespace when the name is literal, and
    /// (None, None) when it's a template.
    fn attribute_key(&self, i: &Instr) -> (Option<String>, Option<String>) {
        let Instr::Attribute {
            name,
            namespace,
            scope,
            ..
        } = i
        else {
            return (None, None);
        };
        let Avt::Literal(n) = name else {
            return (None, None);
        };
        let (prefix, local) = match n.trim().split_once(':') {
            Some((p, l)) => (Some(p), l.to_string()),
            None => (None, n.trim().to_string()),
        };
        let ns = match namespace {
            Some(Avt::Literal(u)) => (!u.is_empty()).then(|| u.clone()),
            Some(Avt::Parts(_)) => None,
            None => prefix
                .and_then(|p| self.resolve_prefix(scope, p))
                .map(|u| self.names.str(u).to_string()),
        };
        (Some(local), ns)
    }

    /// Orders global variables so each comes after the globals it uses.
    /// Later declarations of the same name win.
    pub(super) fn order_globals(&mut self) {
        let globals = std::mem::take(&mut self.ss.globals);
        let mut by_name: HashMap<Name, Variable> = HashMap::new();
        let mut order: Vec<Name> = Vec::new();
        for g in globals {
            if !by_name.contains_key(&g.name) {
                order.push(g.name);
            }
            by_name.insert(g.name, g);
        }
        let mut state: HashMap<Name, u8> = HashMap::new(); // 1 = visiting, 2 = done
        let mut sorted = Vec::new();
        for name in order {
            self.visit_global(name, &by_name, &mut state, &mut sorted);
        }
        self.ss.globals = sorted;
    }

    fn visit_global(
        &mut self,
        name: Name,
        by_name: &HashMap<Name, Variable>,
        state: &mut HashMap<Name, u8>,
        out: &mut Vec<Variable>,
    ) {
        match state.get(&name) {
            Some(2) => return,
            // A circular reference isn't a compile error in libxslt: computing
            // the globals stops the transform (exit 10), which
            // `exec::compute_global` does.
            Some(1) => return,
            _ => {}
        }
        let Some(var) = by_name.get(&name) else {
            return;
        };
        state.insert(name, 1);
        let mut refs = Vec::new();
        refs_in_value(&var.value, &mut refs);
        for r in refs {
            if by_name.contains_key(&r) {
                self.visit_global(r, by_name, state, out);
            }
        }
        state.insert(name, 2);
        out.push(var.clone());
    }
}
