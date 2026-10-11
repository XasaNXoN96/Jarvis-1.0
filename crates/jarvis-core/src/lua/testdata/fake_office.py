#!/usr/bin/env python3
# A fake Word / Excel / Notepad for the automated tests of the document mode (see lua/object_tests.rs).
# It answers the same command line as resources/commands/assistant/office.ps1 and keeps its "programs" in a JSON file
# ($FAKE_STATE). It mirrors the rules of the real script: strict mode never starts a closed program and refuses
# a document other than the one the work mode is bound to.
import base64, json, os, sys

STATE = os.environ["FAKE_STATE"]

def load():
    with open(STATE, encoding="utf-8") as f:
        return json.load(f)

def save(s):
    with open(STATE, "w", encoding="utf-8") as f:
        json.dump(s, f, ensure_ascii=False)

def dec(b):
    return base64.b64decode(b).decode("utf-8") if b else ""

def fail(msg, code):
    sys.stderr.write(msg + "\n")
    sys.exit(code)

def expand(p):
    for tag, d in (("@Documents\\", "/docs/"), ("@Desktop\\", "/desktop/"), ("@Downloads\\", "/downloads/")):
        if p.startswith(tag):
            return d + p[len(tag):]
    return p

def parse(argv):
    a = {"Arg1": "", "Arg2": "", "Arg3": "", "Strict": False, "Expect": ""}
    i = 0
    while i < len(argv):
        k = argv[i]
        if k in ("-App", "-Action", "-Arg1", "-Arg2", "-Arg3", "-Expect"):
            a[k[1:]] = argv[i + 1]; i += 2
        elif k == "-Strict":
            a["Strict"] = True; i += 1
        else:
            i += 1
    return a

def main():
    a = parse(sys.argv[1:])
    s = load()
    app, act = a["App"], a["Action"]
    log = s.setdefault("log", [])
    log.append([app, act, a["Strict"]])

    def prog(name):
        p = s[name]
        if not p["running"]:
            if a["Strict"]:
                fail("NOAPP", 4)
            p["running"] = True
        return p

    def active(p, key):
        items = p[key]
        if not items:
            return None
        return items[p["active"]]

    def check(p, key):
        d = active(p, key)
        if a["Strict"]:
            if d is None:
                fail("NODOC", 5)
            want = dec(a["Expect"])
            if want and want != d["name"]:
                fail("DOCCHANGED:" + d["name"], 6)
        return d

    def info(d):
        if d is None:
            print("0")
        else:
            print("1|%s|%s|%d|%d" % (d["name"], d.get("path") or d["name"], 1 if d.get("saved") else 0, 1 if d.get("path") else 0))

    if app in ("word", "excel"):
        key = "docs" if app == "word" else "books"
        p = s[app]
        if act == "info":
            p = prog(app); info(active(p, key))
        elif act in ("new", "ensure"):
            p = prog(app)
            if act == "new" or not p[key]:
                n = len(p[key]) + 1
                p[key].append({"name": ("Document%d" if app == "word" else "Book%d") % n, "text": "", "fmt": [], "cells": {}, "saved": False})
                p["active"] = len(p[key]) - 1
        elif act == "open":
            p = prog(app); path = dec(a["Arg1"])
            p[key].append({"name": os.path.basename(path), "path": path, "text": s["files"].get(path, ""), "fmt": [], "cells": {}, "saved": True})
            p["active"] = len(p[key]) - 1
        elif act == "text":
            p = prog(app); d = check(p, key)
            if d is None:
                p[key].append({"name": "Document1", "text": "", "fmt": [], "cells": {}, "saved": False}); d = p[key][0]; p["active"] = 0
            d["text"] += dec(a["Arg1"]); d["saved"] = False
        elif act == "format":
            p = prog(app); d = check(p, key); d["fmt"].append(a["Arg1"] + (":" + a["Arg2"] if a["Arg2"] else ""))
        elif act in ("value", "formula"):
            p = prog(app); d = check(p, key)
            if d is None:
                p[key].append({"name": "Book1", "text": "", "fmt": [], "cells": {}, "saved": False}); d = p[key][0]; p["active"] = 0
            d["cells"][a["Arg1"]] = ("=" if False else "") + dec(a["Arg2"]); d["saved"] = False
            if act == "formula":
                print("42")
        elif act == "active":
            p = prog(app); d = check(p, key)
            if d is None:
                p[key].append({"name": "Book1", "text": "", "fmt": [], "cells": {}, "saved": False}); d = p[key][0]; p["active"] = 0
            cell = d.get("active_cell", "A1")
            d["cells"][cell] = dec(a["Arg1"])
            print(cell)
            d["active_cell"] = cell[0] + str(int(cell[1:]) + 1); d["saved"] = False
        elif act == "save":
            p = prog(app); d = check(p, key)
            if not d.get("path"):
                fail("NEEDNAME", 7)
            d["saved"] = True; s["files"][d["path"]] = d["text"]; print(d["path"])
        elif act == "saveas":
            p = prog(app); d = check(p, key); path = expand(dec(a["Arg1"]))
            if path in s["files"] and a["Arg2"] != "overwrite":
                fail("EXISTS", 8)
            d["path"] = path; d["name"] = os.path.basename(path); d["saved"] = True; s["files"][path] = d["text"]; print(path)
        elif act == "discard":
            p = prog(app); d = check(p, key)
            print(d.get("path") or "")
            p[key].pop(p["active"]); p["active"] = len(p[key]) - 1
        else:
            fail("unknown action", 3)
    elif app == "notepad":
        p = s["notepad"]
        if act == "info":
            print("0" if not p["running"] else "1|%s||%d|0" % (p["title"], 0 if p["title"].startswith("*") else 1))
        elif act == "save":
            if not p["running"]:
                fail("NOAPP", 4)
            path = expand(dec(a["Arg1"]))
            if path and path in s["files"] and a["Arg2"] != "overwrite":
                fail("EXISTS", 8)
            if path:
                s["files"][path] = p.get("text", ""); p["title"] = os.path.basename(path)
            print(path)
        elif act == "discard":
            if not p["running"]:
                fail("NOAPP", 4)
            p["running"] = False; print("closed")
        else:
            fail("unknown action", 3)
    elif app == "file":
        if act == "find":
            words = dec(a["Arg1"]).lower().replace("ё", "е").split(); exts = a["Arg2"].split(",")
            hits = [f for f in s["files"] if f.rsplit(".", 1)[-1] in exts and all(w in os.path.basename(f).lower().replace("ё", "е") for w in words)]
            print(hits[0] if hits else "")
        elif act == "recycle":
            path = dec(a["Arg1"])
            if path in s["files"]:
                del s["files"][path]; s.setdefault("recycled", []).append(path); print("recycled")
            else:
                print("absent")
        else:
            fail("unknown action", 3)
    else:
        fail("unknown action", 3)
    save(s)

main()
