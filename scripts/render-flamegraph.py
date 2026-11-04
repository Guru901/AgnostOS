#!/usr/bin/env python3
"""Render invocation counts from manually instrumented AgnostOS functions."""

import html
import sys
from pathlib import Path


def fail(message):
    raise SystemExit(message)


def render(trace, output):
    lines = trace.read_text(errors="replace").splitlines()
    if "AGNOSTOS_PROFILE_V1" not in lines or "AGNOSTOS_PROFILE_END" not in lines:
        fail("profile trace is incomplete or has an unsupported format")

    calls = {}
    dropped = 0
    for line in lines:
        if line.startswith("E:"):
            name = line[2:].strip()
            if name:
                calls[name] = calls.get(name, 0) + 1
        elif line.startswith("AGNOSTOS_PROFILE_DROPPED="):
            try:
                dropped = int(line.split("=", 1)[1])
            except ValueError:
                fail("profile trace has an invalid dropped-call count")

    total = sum(calls.values())
    max_count = max(calls.values(), default=1)
    rows = "".join(
        '<li><div class="row-head"><span class="name">'
        f'{html.escape(name)}</span><span class="count">{count:,} calls · '
        f'{count * 100 / max(total, 1):.1f}%</span></div>'
        f'<div class="track"><div class="bar" style="width:{count * 100 / max_count:.3f}%"></div></div></li>'
        for name, count in sorted(calls.items(), key=lambda item: (-item[1], item[0]))
    )
    if not rows:
        rows = '<li class="empty">No annotated function calls were recorded.</li>'
    warning = (
        f'<p class="warning">The event buffer filled; {dropped:,} later calls were dropped.</p>'
        if dropped
        else ""
    )
    document = f'''<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>AgnostOS Function Profile</title>
<style>
:root{{--ink:#20242c;--muted:#667085;--line:#e4e7ec;--paper:#fff;--wash:#f4f6f8;--accent:#e99a36}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--wash);color:var(--ink);font:14px/1.45 system-ui,-apple-system,"Segoe UI",sans-serif}}
main{{max-width:1200px;margin:0 auto;padding:28px clamp(16px,3vw,40px) 48px}}
.card{{background:var(--paper);border:1px solid var(--line);border-radius:12px;box-shadow:0 1px 2px #1018280a;padding:20px}}
header{{display:flex;justify-content:space-between;align-items:center;gap:16px;margin-bottom:16px}}
h1{{font-size:22px;margin:0 0 4px}}p{{margin:0;color:var(--muted)}}.total{{white-space:nowrap;font-weight:650;border:1px solid var(--line);border-radius:999px;padding:8px 12px}}
ul{{list-style:none;margin:18px 0 0;padding:0}}li+li{{border-top:1px solid #f0f1f3}}li{{padding:12px 0}}.row-head{{display:flex;justify-content:space-between;gap:16px;margin-bottom:7px}}
.name{{font:12px/1.5 ui-monospace,SFMono-Regular,Consolas,monospace;overflow-wrap:anywhere}}.count{{color:var(--muted);white-space:nowrap;font-size:12px}}
.track{{height:10px;background:#f2f4f7;border-radius:6px;overflow:hidden}}.bar{{height:100%;background:var(--accent);border-radius:6px}}
.empty{{color:var(--muted)}}.warning{{margin-top:16px;color:#b54708}}
@media(max-width:620px){{main{{padding:16px 10px}}.card{{padding:14px}}header{{align-items:flex-start;flex-direction:column}}.row-head{{align-items:flex-start;flex-direction:column;gap:2px}}}}
</style></head><body><main><header><div><h1>AgnostOS function profile</h1>
<p>Invocation counts from functions explicitly marked with <code>#[profiled]</code>.</p></div>
<div class="total">{total:,} recorded calls</div></header>
<section class="card" aria-label="Function invocation counts"><ul>{rows}</ul>{warning}</section>
</main></body></html>'''
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(document)
    print(f"Rendered {total} manually recorded function calls to {output}")


def main():
    if len(sys.argv) != 3:
        fail("usage: render-flamegraph.py PROFILE_TRACE OUTPUT_HTML")
    render(Path(sys.argv[1]), Path(sys.argv[2]))


if __name__ == "__main__":
    main()
