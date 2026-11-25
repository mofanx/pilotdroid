#!/usr/bin/env python3
"""Mini XPath evaluator over a uiautomator dump ElementTree
(stdlib-only, Track B: `_XPathLite` 常见面全量升级).

Supported (the u2-common surface — see compat/SUPPORTED.md):
- `//tag`, `/tag`, `*`, `.`/`..` steps
- axes: child, descendant, ancestor, ancestor-or-self, parent, self,
  following-sibling, preceding-sibling
- predicates: `[@a='v']` `[@a]` `[@a!='v']` `[n]` (1-based, per
  context node) `and`/`or` (standard precedence) `contains/starts-
  with/ends-with(@a,'v')` and `not(...)` around those
- union `|` of two paths

Out of scope → NotImplementedError with the offending construct
named: last()/position()/text()/comment()/namespace/following|preceding
(non-sibling)/attribute::/numeric <,>,<=,>= operators/functions other
than the three above.
"""
import re

_AXES = ("ancestor-or-self", "ancestor", "following-sibling",
         "preceding-sibling", "descendant-or-self", "descendant",
         "child", "parent", "self")


class _NIE(NotImplementedError):
    pass


def _split_top(x, sep):
    """Split `x` on `sep` at bracket/paren depth 0."""
    out, dep, i = [], 0, 0
    while i < len(x):
        c = x[i]
        if c in "[(":
            dep += 1
        elif c in "])":
            dep -= 1
        elif c == sep and dep == 0:
            out.append(x[:i])
            x = x[i + 1:]
            i = -1
        i += 1
    out.append(x)
    return out


# ---------- predicate parser ----------
# pred := or_expr
# or_expr := and_expr ("or" and_expr)*
# and_expr := unary ("and" unary)*
# unary := "not(" or_expr ")" | test
# test := "@name" (("="|"!=") lit)? | func "(" "@name" "," lit ")"
#        | number (position, handled by caller)

def _lex_pred(s):
    toks, i = [], 0
    while i < len(s):
        c = s[i]
        if c.isspace():
            i += 1
        elif c in "[],()=@!":
            toks.append(c)
            i += 1
        elif c in "'\"":
            j = s.index(c, i + 1)
            toks.append(s[i:j + 1])
            i = j + 1
        else:
            j = i
            while j < len(s) and s[j] not in "[],()=@!'\" \t":
                j += 1
            toks.append(s[i:j])
            i = j
    return toks


class _P:
    def __init__(self, toks):
        self.t, self.i = toks, 0

    def peek(self):
        return self.t[self.i] if self.i < len(self.t) else None

    def eat(self, want=None):
        tok = self.peek()
        if want and tok != want:
            raise _NIE(f"bad predicate syntax: want {want!r} got {tok!r}")
        self.i += 1
        return tok


def _attr_val(el, name):
    return el.get(name) or ""


def _parse_pred(s):
    """-> predicate fn(el)->bool, or int for a bare [n] position."""
    s = s.strip()
    if re.fullmatch(r"\d+", s):
        return int(s)
    toks = _lex_pred(s)
    p = _P(toks)
    fn = _or(p)
    if p.peek() is not None:
        raise _NIE(f"trailing tokens in predicate [{s}]")
    return fn


def _lit(p):
    tok = p.eat()
    if not tok or tok[0] not in "'\"":
        raise _NIE(f"expected literal, got {tok!r}")
    return tok[1:-1]


def _primary(p):
    tok = p.eat()
    if tok == "@":
        name = p.eat()
        if p.peek() == "=":
            p.eat()
            v = _lit(p)
            return lambda el: _attr_val(el, name) == v
        if p.peek() == "!" or p.peek() == "!=":
            p.eat()
            p.eat("=")  # '!' and '=' lex as separate tokens
            v = _lit(p)
            # XPath: @a!='v' is false when @a is ABSENT (node-set
            # comparison) — not(@a='v') would be true. Keep them apart.
            return lambda el: el.get(name) is not None \
                and _attr_val(el, name) != v
        return lambda el: el.get(name) is not None
    if tok in ("contains", "starts-with", "ends-with"):
        p.eat("(")
        p.eat("@")
        name = p.eat()
        p.eat(",")
        v = _lit(p)
        p.eat(")")
        if tok == "contains":
            return lambda el: v in _attr_val(el, name)
        if tok == "starts-with":
            return lambda el: _attr_val(el, name).startswith(v)
        return lambda el: _attr_val(el, name).endswith(v)
    if re.fullmatch(r"[\w.-]+", tok or "") and \
            tok.split("-")[0].isidentifier():
        raise _NIE(f"unsupported predicate construct {tok!r} in xpath")
    raise _NIE(f"unsupported predicate syntax near {tok!r}")


def _unary(p):
    if p.peek() == "not" and p.i + 1 < len(p.t) and p.t[p.i + 1] == "(":
        p.eat(); p.eat()
        fn = _or(p)
        p.eat(")")
        return lambda el: not fn(el)
    return _primary(p)


def _and(p):
    fn = _unary(p)
    while p.peek() == "and":
        p.eat()
        g = _unary(p)
        fn = (lambda f=fn, h=g: lambda el: f(el) and h(el))()
    return fn


def _or(p):
    fn = _and(p)
    while p.peek() == "or":
        p.eat()
        g = _and(p)
        fn = (lambda f=fn, h=g: lambda el: f(el) or h(el))()
    return fn


# ---------- step parser ----------

def _split_steps(x):
    """'//a/b[2]//c' -> [('desc','a'),('child','b[2]'),('desc','c')].
    Leading '//' = descendant search from root."""
    steps, i, dep = [], 0, 0
    buf = ""
    cur = None  # None until first separator
    while i < len(x):
        c = x[i]
        if c == "[":
            dep += 1
        elif c == "]":
            dep -= 1
        if c == "/" and dep == 0:
            if buf.strip():
                steps.append((cur or "child", buf.strip()))
                buf = ""
            nxt = x[i + 1] if i + 1 < len(x) else ""
            cur = "desc" if nxt == "/" else "child"
            if nxt == "/":
                i += 1
            i += 1
            continue
        buf += c
        i += 1
    if buf.strip():
        steps.append((cur or "child", buf.strip()))
    return steps


_STEP_RE = re.compile(
    r"^(?:(ancestor-or-self|ancestor|following-sibling|"
    r"preceding-sibling|descendant-or-self|descendant|child|"
    r"parent|self)::)?(\*|[\w.-]+|\.\.)?(.*)$")


def _parse_step(s):
    m = _STEP_RE.match(s)
    if not m:
        raise _NIE(f"bad step {s!r}")
    axis, tag, rest = m.groups()
    tag = tag or "*"
    if tag == "..":
        axis, tag = "parent", "*"
    elif tag == ".":
        axis, tag = "self", "*"
    preds = []
    for pm in re.finditer(r"\[([^\]]*)\]", rest or ""):
        preds.append(_parse_pred(pm.group(1)))
    rest_n = re.sub(r"\[[^\]]*\]", "", rest or "").strip()
    if rest_n:
        raise _NIE(f"unsupported step fragment {rest_n!r} in {s!r}")
    return axis, tag, preds


# ---------- evaluator ----------

def _axis_nodes(root, pmap, ctx, axis):
    if axis == "self":
        return [ctx]
    if axis == "parent":
        p = pmap.get(ctx)
        return [p] if p is not None else []
    if axis == "child":
        return list(ctx)
    if axis == "descendant":
        return [e for e in ctx.iter() if e is not ctx]
    if axis in ("ancestor", "ancestor-or-self"):
        out, p = ([ctx] if axis.endswith("-or-self") else []), pmap.get(ctx)
        while p is not None:
            out.append(p)
            p = pmap.get(p)
        return out
    if axis in ("following-sibling", "preceding-sibling"):
        p = pmap.get(ctx)
        if p is None:
            return []
        sibs = list(p)
        i = sibs.index(ctx)
        return sibs[i + 1:] if axis == "following-sibling" else sibs[:i]
    raise _NIE(f"unsupported axis {axis!r}")


def eval_xpath(root, expr):
    """-> [Element] in document order. Raises _NIE on out-of-scope."""
    x = expr.strip()
    if x.startswith("."):
        x = x.lstrip(".")
    if not x.startswith("/"):
        raise _NIE(f"xpath must start with / or //: {expr!r}")
    for bad in ("last()", "position()", "text()", "comment()",
                "namespace::", "attribute::", "following::",
                "preceding::", "<=", ">=", "' <", "' >"):
        if bad in x:
            raise _NIE(f"xpath-lite: {bad.strip()!r} unsupported "
                       f"(in {expr!r})")
    pmap = {c: p for p in root.iter() for c in p}
    results = []
    for part in _split_top(x, "|"):
        ctxs = [root]
        for sep, raw in _split_steps(part):
            axis, tag, preds = _parse_step(raw)
            if sep == "desc" and axis is None:
                # `//tag` = descendants of each ctx (incl. ctx? no —
                # /descendant-or-self::node()/child::tag: tag children
                # of the set; equivalent to all descendant matches)
                axis = "descendant-all"
            axis = axis or "child"
            out = []
            for ctx in ctxs:
                if axis == "descendant-all":
                    cand = [e for e in ctx.iter() if e is not ctx]
                else:
                    cand = _axis_nodes(root, pmap, ctx, axis)
                cand = [e for e in cand if tag in ("*", e.tag)]
                if axis == "descendant-all" and \
                        any(isinstance(pr, int) for pr in preds):
                    # `//x[n]` is per spec /descendant-or-self::node()/
                    # child::x[n] — position counts within each parent's
                    # child set, not the flat document-order list.
                    groups = {}
                    for e in cand:
                        groups.setdefault(id(pmap.get(e)), []).append(e)
                    keep = set()
                    for g in groups.values():
                        for pr in preds:
                            if isinstance(pr, int):
                                g = g[pr - 1:pr] if 0 < pr <= len(g) \
                                    else []
                            else:
                                g = [e for e in g if pr(e)]
                        keep.update(id(e) for e in g)
                    cand = [e for e in cand if id(e) in keep]
                else:
                    for pr in preds:
                        if isinstance(pr, int):
                            cand = cand[pr - 1:pr] if 0 < pr <= len(cand) \
                                else []
                        else:
                            cand = [e for e in cand if pr(e)]
                out.extend(cand)
            ctxs = out
        results.extend(ctxs)
    return results
