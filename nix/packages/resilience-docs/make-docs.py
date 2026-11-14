"""Build the HTML tree of the built-in documentation pack (ADR 0013).

Inputs (all from the Nix store): the Resilience handbook (Markdown), the
NixOS manual (HTML, three large pages) and the NixOS options (JSON). The
options are split into one page per group (first two name components), so
every page stays small enough for a text browser and full-text search
returns useful hits. Only the standard library is used.

usage: make-docs.py HANDBOOK_DIR MANUAL_DIR OPTIONS_JSON OUT_DIR
"""

import html
import json
import os
import re
import sys
from collections import defaultdict

handbook_dir, manual_dir, options_json, out = sys.argv[1:5]
os.makedirs(os.path.join(out, "options"), exist_ok=True)
os.makedirs(os.path.join(out, "handbook"), exist_ok=True)


def page(title, body):
    return (
        '<!DOCTYPE html><html><head><meta charset="utf-8"><title>'
        + html.escape(title)
        + "</title></head><body>\n"
        + body
        + "\n</body></html>\n"
    )


def write(rel, text):
    with open(os.path.join(out, rel), "w", encoding="utf-8") as f:
        f.write(text)


def group_of(name):
    parts = name.split(".")
    return ".".join(parts[:2]) if len(parts) > 2 else parts[0]


def safe(name):
    return re.sub(r"[^A-Za-z0-9._-]", "_", name)


# --- handbook ---------------------------------------------------------------
hb = []
for f in sorted(os.listdir(handbook_dir)):
    if not f.endswith(".md"):
        continue
    text = open(os.path.join(handbook_dir, f), encoding="utf-8").read()
    first = text.splitlines()[0] if text else f
    title = first.lstrip("# ").strip() or f
    rel = "handbook/" + safe(f[:-3]) + ".html"
    write(rel, page(title, "<h1>" + html.escape(title) + "</h1>\n<pre>"
                    + html.escape("\n".join(text.splitlines()[1:])) + "</pre>"))
    hb.append((title, rel))

# --- NixOS manual: drop scripts and styles, point option links at our pages --
def clean(doc):
    doc = re.sub(r"<script\b.*?</script>", "", doc, flags=re.S | re.I)
    doc = re.sub(r"<link\b[^>]*>", "", doc, flags=re.I)

    def opt_link(m):
        name = html.unescape(m.group(1))
        return 'href="options/%s.html#opt-%s"' % (safe(group_of(name)), m.group(1))

    return re.sub(r'href="options\.html#opt-([^"]+)"', opt_link, doc)


for name in ["index", "release-notes"]:
    src = os.path.join(manual_dir, name + ".html")
    doc = open(src, encoding="utf-8").read()
    write("nixos-" + name + ".html", clean(doc).replace('href="release-notes.html', 'href="nixos-release-notes.html'))

# --- NixOS options ------------------------------------------------------------
opts = json.load(open(options_json, encoding="utf-8"))
groups = defaultdict(list)
for name in sorted(opts):
    groups[group_of(name)].append(name)


def show(v):
    if isinstance(v, dict) and "text" in v:
        return v["text"]
    return json.dumps(v)


for g, names in groups.items():
    parts = ["<h1>NixOS options: " + html.escape(g) + "</h1>",
             '<p><a href="../options.html">All option groups</a></p>']
    for n in names:
        o = opts[n]
        parts.append('<h2 id="opt-%s">%s</h2>' % (html.escape(n), html.escape(n)))
        parts.append("<p>" + html.escape(o.get("description") or "").replace("\n\n", "</p><p>") + "</p>")
        parts.append("<p>Type: " + html.escape(o.get("type", "?")) + "</p>")
        if "default" in o:
            parts.append("<p>Default:</p><pre>" + html.escape(show(o["default"])) + "</pre>")
        if "example" in o:
            parts.append("<p>Example:</p><pre>" + html.escape(show(o["example"])) + "</pre>")
        decl = ", ".join(o.get("declarations", []))
        if decl:
            parts.append("<p>Declared in: " + html.escape(decl) + "</p>")
    write("options/" + safe(g) + ".html", page("NixOS options: " + g, "\n".join(parts)))

by_top = defaultdict(list)
for g in sorted(groups):
    by_top[g.split(".")[0]].append(g)
idx = ["<h1>NixOS options</h1>",
       "<p>%d options of NixOS, in %d groups. Search finds an option by name or by words "
       "of its description.</p>" % (len(opts), len(groups))]
for top, gs in sorted(by_top.items()):
    idx.append("<h2>" + html.escape(top) + "</h2><p>" + " | ".join(
        '<a href="options/%s.html">%s</a> (%d)' % (safe(g), html.escape(g), len(groups[g]))
        for g in gs) + "</p>")
write("options.html", page("NixOS options", "\n".join(idx)))

# --- welcome page -------------------------------------------------------------
welcome = [
    "<h1>System documentation</h1>",
    "<p>Offline documentation of this computer: the Resilience handbook, the NixOS "
    "manual and every NixOS configuration option. Use s in the Offline library to "
    "search all of it.</p>",
    "<h2>Resilience handbook</h2><ul>"
    + "".join('<li><a href="%s">%s</a></li>' % (rel, html.escape(t)) for t, rel in hb)
    + "</ul>",
    '<h2>NixOS</h2><ul><li><a href="nixos-index.html">NixOS manual</a></li>'
    '<li><a href="options.html">NixOS configuration options</a></li>'
    '<li><a href="nixos-release-notes.html">NixOS release notes</a></li></ul>',
    "<p>Sources: the Resilience handbook of this system; the NixOS manual and option "
    "list of the pinned nixpkgs (MIT license, nixos.org). Built from the system's own "
    "sources, so it matches this system.</p>",
]
write("index.html", page("System documentation", "\n".join(welcome)))
