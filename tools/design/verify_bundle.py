#!/usr/bin/env python3
"""Verify a Claude Design bundle before importing it. Reads files as DATA only; nothing from the
bundle is executed.

Usage: verify_bundle.py BUNDLE_DIR [COMMITTED_DESIGN_DIR]

BUNDLE_DIR is the unzipped bundle. COMMITTED_DESIGN_DIR (default docs/soundcraft-ai/design) holds the
icons.json and messages.json already in the repository, so the script can tell which icons and
messages CHANGED, and whether the fixes asked for in the last review landed.

Exit code 0 when no check FAILs (WARN and MANUAL items are listed but do not fail).
"""
import ast, glob, json, os, re, sys
import xml.etree.ElementTree as ET

bundle = sys.argv[1]
committed = sys.argv[2] if len(sys.argv) > 2 else 'docs/soundcraft-ai/design'
results = []  # (status, check, detail)
def add(status, check, detail=''): results.append((status, check, detail))

def lit(src): return ast.literal_eval(src)
def bracket(text, start):
    opener = text[start]; closer = {'[': ']', '{': '}'}[opener]; depth = 0; quote = None; j = start
    while j < len(text):
        c = text[j]
        if quote:
            if c == '\\': j += 1
            elif c == quote: quote = None
        elif c in '\'"`': quote = c
        elif c == opener: depth += 1
        elif c == closer:
            depth -= 1
            if depth == 0: return text[start:j + 1]
        j += 1
    raise ValueError('unbalanced')
def script(path):
    s = open(path, encoding='utf-8').read(); i = s.find('data-dc-script'); return s, s[i:s.find('</script>', i)]
def const(blk, name):
    m = re.search(r'const %s\s*=\s*' % re.escape(name), blk)
    if not m: raise KeyError(name)
    return bracket(blk, m.end())

# ---- icons ----
data_js = f'{bundle}/icons-data.js'
rows = []
if not os.path.exists(data_js):
    add('FAIL', 'icons-data.js present')
else:
    src = open(data_js, encoding='utf-8').read()
    rows = lit(src[src.index('['):src.rindex(']') + 1])
    byname = {r[0]: r for r in rows}
    add('PASS' if len(rows) >= 63 else 'FAIL', 'icon count', f'{len(rows)} icons (the brief needs about 60)')
    nums = lambda d: [float(x) for x in re.findall(r'-?\d*\.?\d+', d)]
    scaled = []
    for r in rows:
        x, y = nums(r[2]), nums(r[4])
        if x and len(x) == len(y):
            ratio = [b / a for a, b in zip(x, y) if abs(a) > 1e-9]
            if ratio and max(ratio) - min(ratio) < 0.02 and abs(sum(ratio) / len(ratio) - 0.8) < 0.03: scaled.append(r[0])
    add('PASS' if len(scaled) <= 5 else 'FAIL', '16 px redrawn, not scaled', f'{len(scaled)} icons are exactly 0.8x of the 20 px geometry {scaled[:8]}')
    vals = [v for r in rows for v in nums(r[4])]
    snapped = sum(1 for v in vals if abs(v - round(v)) < 1e-6) / max(1, len(vals))
    add('PASS' if snapped >= 0.95 else 'WARN', '16 px coordinates on whole pixels', f'{snapped:.0%}')
    # SVG files
    bad, mism, missing = [], [], []
    for r in rows:
        for size, d in (('20', r[2]), ('16', r[4])):
            f = f'{bundle}/icons/{r[0]}.{size}.svg'
            if not os.path.exists(f): missing.append(os.path.basename(f)); continue
            try: root = ET.parse(f).getroot()
            except ET.ParseError: bad.append(f'{os.path.basename(f)}: not XML'); continue
            for el in root.iter():
                tag = el.tag.split('}')[-1]
                if tag in ('script', 'foreignObject', 'use', 'image', 'style'): bad.append(f'{os.path.basename(f)}: <{tag}>')
                for a in el.attrib:
                    a2 = a.split('}')[-1]
                    if a2.startswith('on') or a2 in ('href', 'src'): bad.append(f'{os.path.basename(f)}: {a2}')
            if root.attrib.get('viewBox') != f'0 0 {size} {size}': bad.append(f'{os.path.basename(f)}: viewBox')
            ds = [e.attrib.get('d') for e in root.iter() if e.tag.endswith('path')]
            if d and d not in ds and ' '.join(ds) != d: mism.append(os.path.basename(f))
    add('PASS' if not missing else 'FAIL', 'both sizes exist for every icon', f'missing {missing[:5]}' if missing else '')
    add('PASS' if not bad else 'FAIL', 'SVGs are safe (no script, handlers, links, images, styles; right viewBox)', '; '.join(bad[:5]))
    add('PASS' if not mism else 'FAIL', 'SVG paths match the data table', f'{len(mism)} differ {mism[:4]}' if mism else '')
    # which icons changed since the committed set, and did the requested fixes land
    prev_path = f'{committed}/icons.json'
    if os.path.exists(prev_path):
        prev = {i['name']: i for i in json.load(open(prev_path, encoding='utf-8'))['icons']}
        changed = [r[0] for r in rows if r[0] in prev and (r[2], r[3], r[4], r[5]) != (prev[r[0]]['size_20']['stroke'], prev[r[0]]['size_20']['fill'], prev[r[0]]['size_16']['stroke'], prev[r[0]]['size_16']['fill'])]
        add('INFO', 'icons changed since the committed set', f'{len(changed)}: {changed}')
        for name, why in (('zoom', 'distinct from zoom-out at 20 px'), ('smart-tool', 'no sparkle'), ('stale', 'not a sigma')):
            add('PASS' if name in changed else 'FAIL', f'requested fix landed: {name} ({why})', '' if name in changed else 'geometry is unchanged')
        add('MANUAL', 'look at the three replaced icons', 'render them: a shape change does not prove the new shape is a good one')
    if 'zoom' in byname and 'zoom-out' in byname:
        a, b = byname['zoom'][2], byname['zoom-out'][2]
        add('PASS' if a != b and nums(a) != nums(b) else 'FAIL', 'zoom differs from zoom-out at 20 px', '')
        # a 20 px zoom whose only difference from zoom-out is a shorter bar is the known near-duplicate
        # the known near-duplicate: same lens and handle, only a shorter bar, and no other mark
        diffbar = re.sub(r'M\d+ 9h\d+$', '', a) == re.sub(r'M\d+ 9h\d+$', '', b) and not byname['zoom'][3]
        add('WARN' if diffbar else 'PASS', 'zoom is more than zoom-out with a shorter bar', 'only the bar length differs' if diffbar else '')

# ---- fader law and labels ----
def real_law(db):
    if db <= -144: return 0.0
    if db >= -24: return min(1.0, 0.25 + (min(db, 12) + 24) / 36 * 0.75)
    x = min(1, max(0, (-24 - db) / 120)) ** (1 / 1.5); return max(0.0, (1 - x) * 0.25)
page6 = f'{bundle}/Round 6 Review Messages Fader.dc.html'
if os.path.exists(page6):
    text6, b6 = script(page6)
    try:
        pts = lit(const(b6, 'pts').replace('-Infinity', '-1e9'))
        worst = max(abs(real_law(-144 if d < -1e8 else d) - f) for d, f in pts)
        add('PASS' if worst < 0.002 else 'FAIL', "fader law matches the app's fader_db_to_pos", f'{len(pts)} points, largest difference {worst:.4f}')
        table = dict(pts)
        for h, labels in re.findall(r'mkF\((\d+), \[([^\]]*)\]', b6):
            ds = [float(x) if '-Infinity' not in x else -1e9 for x in labels.split(',')]
            ys = [18 + (1 - table[d if d in table else min(table, key=lambda k: abs(k - d))]) * (int(h) - 36) for d in ds]
            gap = min(b - a for a, b in zip(ys, ys[1:]))
            add('PASS' if gap >= 11 else 'FAIL', f'fader labels at {h} px do not overlap (11 px text)', f'tightest gap {gap:.1f} px')
    except Exception as e:
        add('FAIL', 'fader law readable', str(e))
    # messages
    try:
        a_val = re.search(r"const A = '([^']*)'", b6).group(1)
        msrc = re.sub(r'\bM\(', '(', const(b6, 'msgs')).replace(', A)', f", '{a_val}')")
        msgs = {k: (t, w, where) for k, t, w, where in lit(msrc)}
        banned = re.compile(r'\b(error|invalid|ai-powered|smart|magic|better|correct)\b|\d+\s?%', re.I)
        info = {'limit_change_size', 'limit_fader_range', 'clipped_lowering_note', 'preview_unavailable_plugin'}
        problems, warns = [], []
        for k, (t, w, where) in msgs.items():
            if banned.search(t + ' ' + w): problems.append(f'{k}: banned word')
            if not w.strip(): problems.append(f'{k}: no what-you-can-do')
            n = len(re.findall(r'[.!?](\s|$)', t))
            if n > 2: warns.append(f'{k}: {n} sentences')
            if k not in info and 'hang' not in t + w: problems.append(f'{k}: does not say whether anything changed')
            if text6.count(t) < 1: problems.append(f'{k}: text not in the source page')
        add('PASS' if not problems else 'FAIL', 'messages follow the style rules', '; '.join(problems))
        add('WARN' if warns else 'PASS', 'messages are two sentences or fewer', '; '.join(warns))
        add('PASS' if len(msgs) >= 15 else 'FAIL', 'message count', f'{len(msgs)} (the round 6 brief needs 11 situations plus 3 strings)')
        p = msgs.get('preview_unavailable_plugin')
        if p: add('PASS' if re.search(r'ask again|stale|new proposal', p[0] + ' ' + p[1], re.I) else 'FAIL', 'requested fix landed: the plug-in message says asking again is needed', p[1])
        g = msgs.get('goal_unsupported')
        if g:
            hard = all(x in (g[0] + ' ' + g[1]) for x in ('Headroom', 'Vocal clarity', 'Even level'))
            add('FAIL' if hard else 'PASS', 'requested fix landed: unsupported-goal message does not hard-code all three goals', 'still lists all three' if hard else '')
    except Exception as e:
        add('FAIL', 'messages readable', str(e))
else:
    add('FAIL', 'Round 6 messages and fader page present')

# ---- the other asked-for items that need eyes ----
add('MANUAL', 'board 3d: focus ring on the A/B switch is drawn', 'look at the render')
add('MANUAL', 'board 6e: the 260 px caption matches the drawn labels', 'look at the render')
add('MANUAL', 'latch versus touch at 16 px, and 1 px strokes at 20 px, are legible at 1x', 'look at a 1x render')

width = max(len(c) for _, c, _ in results)
order = {'FAIL': 0, 'WARN': 1, 'MANUAL': 2, 'INFO': 3, 'PASS': 4}
for status, check, detail in sorted(results, key=lambda r: order[r[0]]):
    print(f'{status:6} {check:<{min(width, 88)}}  {detail}')
fails = [r for r in results if r[0] == 'FAIL']
print(f'\n{len(fails)} FAIL, {sum(1 for r in results if r[0] == "WARN")} WARN, {sum(1 for r in results if r[0] == "MANUAL")} MANUAL, {sum(1 for r in results if r[0] == "PASS")} PASS')
sys.exit(1 if fails else 0)
