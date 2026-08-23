#!/usr/bin/env python3
"""Fails if a scan report disagrees with the reference on any decision.

Compares group membership and the keeper chosen under every rule. Confidence
values are reported but not asserted: they depend on decoder alignment, which
differs legitimately between platforms, while the decisions must not.
"""
import json, sys

def key(report):
    return {(g["kind"], tuple(sorted(g["paths"]))): g for g in report["groups"]}

expected = key(json.load(open(sys.argv[1])))
actual   = key(json.load(open(sys.argv[2])))

problems = []
for g in sorted(set(expected) - set(actual)):
    problems.append(f"missing group: {g[0]} {list(g[1])}")
for g in sorted(set(actual) - set(expected)):
    problems.append(f"unexpected group: {g[0]} {list(g[1])}")
for g in sorted(set(expected) & set(actual)):
    for rule, keeper in expected[g]["keepers"].items():
        got = actual[g]["keepers"].get(rule)
        if got != keeper:
            problems.append(f"keeper differs {list(g[1])} [{rule}]: expected {keeper}, got {got}")

print(f"{len(actual)} groups checked against the reference")
for p in problems:
    print("  " + p)
sys.exit(1 if problems else 0)
