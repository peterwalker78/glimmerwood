#!/usr/bin/env python3
"""Carry the welcome page's data across from core/data.

The page has no core to ask, so the good places and the facts travel with it.
Only what the page needs comes across: it has no dose to weigh places against.
"""

import json
import sys
import tomllib
from pathlib import Path

root = Path(__file__).resolve().parent.parent
pool = tomllib.loads((root / "core/data/places.toml").read_text())

places = [
    {
        "name": p["name"],
        "line": p["line"],
        "url": p["url"],
        "kind": p["kinds"][0],
        "moments": p["moments"],
        "seasons": p.get("seasons", []),
        "fresh": p.get("fresh", False),
        "uk": p["region"] == "uk",
    }
    for p in pool["places"]
]

rows = ",\n".join("  " + json.dumps(p, ensure_ascii=False) for p in places)
out = f"""// Generated from core/data/places.toml by scripts/site. Do not edit.

export type SitePlace = {{
  name: string;
  line: string;
  url: string;
  // The main good it does, used to keep a set of six mixed.
  kind: string;
  moments: string[];
  // Months it suits, 1-12; empty for all year.
  seasons: number[];
  // Its content renews daily or weekly, so it can come round sooner.
  fresh: boolean;
  // Only offered to someone in the UK.
  uk: boolean;
}};

// Hours in each stretch of the day between new picks.
export const ROTATE_HOURS = {pool["rotate_hours"]};

export const PLACES: SitePlace[] = [
{rows},
];
"""

target = root / "ui/site/places.gen.ts"
target.write_text(out)
print(f"site: {len(places)} places -> {target.relative_to(root)}", file=sys.stderr)

facts = tomllib.loads((root / "core/data/facts.toml").read_text())["facts"]
rows = ",\n".join("  " + json.dumps(f, ensure_ascii=False) for f in facts)
out = f"""// Generated from core/data/facts.toml by scripts/site. Do not edit.

export type Fact = {{
  text: string;
  source: string;
  url: string;
}};

export const FACTS: Fact[] = [
{rows},
];
"""
target = root / "ui/site/facts.gen.ts"
target.write_text(out)
print(f"site: {len(facts)} facts -> {target.relative_to(root)}", file=sys.stderr)
