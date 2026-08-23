#!/usr/bin/env python3
"""Tier 3: compares the Swift and Rust scan reports for identical decisions."""
import json, subprocess, sys, os

SWIFT = "./build/paritydump"
RUST  = "rust/target/release/ddcli"

def swift_scan(d, s):
    return json.loads(subprocess.run([SWIFT, "scan", d, str(s)],
                                     capture_output=True, text=True).stdout)

def rust_scan(d, s):
    return json.loads(subprocess.run([RUST, "scan", d, "--sensitivity", str(s), "--json"],
                                     capture_output=True, text=True).stdout)

def key(groups):
    return {(g["kind"], tuple(g["paths"])): g for g in groups}

def compare(name, d, sensitivities):
    problems = []
    for s in sensitivities:
        a, b = swift_scan(d, s), rust_scan(d, s)
        ka, kb = key(a["groups"]), key(b["groups"])
        only_swift = set(ka) - set(kb)
        only_rust  = set(kb) - set(ka)
        for g in sorted(only_swift):
            problems.append(f"  s={s} only in Swift: {g[0]} {list(g[1])}")
        for g in sorted(only_rust):
            problems.append(f"  s={s} only in Rust : {g[0]} {list(g[1])}")
        for g in sorted(set(ka) & set(kb)):
            for rule, keeper in ka[g]["keepers"].items():
                if kb[g]["keepers"].get(rule) != keeper:
                    problems.append(f"  s={s} keeper differs {list(g[1])} [{rule}]: "
                                    f"swift={keeper} rust={kb[g]['keepers'].get(rule)}")
            ca, cb = ka[g]["confidence"], kb[g]["confidence"]
            if abs(ca - cb) > 0.15:
                problems.append(f"  s={s} confidence {list(g[1])}: swift={ca} rust={cb}")
            elif abs(ca - cb) > 0.02:
                print(f"  note s={s} confidence differs by {abs(ca-cb):.3f} "
                      f"{list(g[1])}: swift={ca} rust={cb}")
    status = "MATCH" if not problems else "DIFFERS"
    print(f"{name:<12} {status}")
    for p in problems[:12]:
        print(p)
    return not problems

if __name__ == "__main__":
    base = sys.argv[1] if len(sys.argv) > 1 else "testdata"
    sens = [0.0, 0.35, 0.5, 1.0]
    if os.path.isdir(os.path.join(base, "fixtures")):
        ok = compare("fixtures", os.path.join(base, "fixtures"), sens)
    else:
        ok = True
        for c in ["testaudio", "distinct", "tones", "mixed", "codectest",
                  "m4atest", "durtest", "excerpt"]:
            d = os.path.join(base, c)
            if os.path.isdir(d):
                ok &= compare(c, d, sens)
    sys.exit(0 if ok else 1)
