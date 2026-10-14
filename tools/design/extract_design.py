#!/usr/bin/env python3
"""Extract the design data from the Claude Design bundle as PLAIN DATA (no page code is run).

Reads the JavaScript array literals in the bundle's pages, turns them into Python literals with
ast.literal_eval (which cannot execute code), recomputes contrast ratios and the clip palette with
the same published formulas (WCAG relative luminance; OKLCH to sRGB), and writes tokens.json and
copy.json. Usage: extract_design.py BUNDLE_DIR OUT_DIR
"""
import ast, json, math, re, sys

bundle, out = sys.argv[1], sys.argv[2]

def block(text, name):
    """The bracketed literal assigned to `const NAME =`, found by bracket matching."""
    m = re.search(r'const %s\s*=\s*' % re.escape(name), text)
    if not m:
        raise SystemExit(f"missing const {name}")
    i = m.end()
    opener = text[i]
    closer = {'[': ']', '{': '}'}[opener]
    depth, j, quote = 0, i, None
    while j < len(text):
        c = text[j]
        if quote:
            if c == '\\': j += 1
            elif c == quote: quote = None
        elif c in '\'"`': quote = c
        elif c == opener: depth += 1
        elif c == closer:
            depth -= 1
            if depth == 0: return text[i:j + 1]
        j += 1
    raise SystemExit(f"unbalanced {name}")

def lit(src):
    """JS literal to Python. true/false/null are rewritten ONLY outside string literals."""
    words = {'true': 'True', 'false': 'False', 'null': 'None'}
    out, i, quote = [], 0, None
    while i < len(src):
        c = src[i]
        if quote:
            out.append(c)
            if c == '\\' and i + 1 < len(src):
                i += 1; out.append(src[i])
            elif c == quote:
                quote = None
            i += 1
        elif c in '\'"':
            quote = c; out.append(c); i += 1
        else:
            m = re.match(r'(true|false|null)\b', src[i:]) if (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == '_')) else None
            if m:
                out.append(words[m.group(1)]); i += len(m.group(1))
            else:
                out.append(c); i += 1
    return ast.literal_eval(''.join(out))

ds = open(f'{bundle}/Design System.dc.html', encoding='utf-8').read()
nc = open(f'{bundle}/Notes and Copy.dc.html', encoding='utf-8').read()

# ---- contrast and colour maths (WCAG 2.x luminance; OKLCH -> sRGB) ----
def rgb(h): return [int(h[i:i + 2], 16) / 255 for i in (1, 3, 5)]
def lum(h):
    r, g, b = [v / 12.92 if v <= .03928 else ((v + .055) / 1.055) ** 2.4 for v in rgb(h)]
    return .2126 * r + .7152 * g + .0722 * b
def cr(a, b):
    x, y = lum(a), lum(b)
    return (max(x, y) + .05) / (min(x, y) + .05)
def ok2h(L, C, H):
    a, b = C * math.cos(math.radians(H)), C * math.sin(math.radians(H))
    l = (L + .3963377774 * a + .2158037573 * b) ** 3
    m = (L - .1055613458 * a - .0638541728 * b) ** 3
    s = (L - .0894841775 * a - 1.291485548 * b) ** 3
    lin = [4.0767416621 * l - 3.3077115913 * m + .2309699292 * s, -1.2684380046 * l + 2.6097574011 * m - .3413193965 * s, -.0041960863 * l - .7034186147 * m + 1.707614701 * s]
    def enc(v):
        v = max(0, min(1, v)); v = 12.92 * v if v <= .0031308 else 1.055 * v ** (1 / 2.4) - .055
        return round(v * 255)
    return '#' + ''.join(f'{enc(v):02x}' for v in lin)

def tokens(rows, surface):
    res = []
    for r in rows:
        name, hexv, use = r[0], r[1], r[2]
        ag = r[3] if len(r) > 3 else None
        t = {'name': name, 'value': hexv, 'usage': use}
        if ag:
            against = surface if ag == 1 else ag
            t['contrast_against'] = against
            t['contrast'] = round(cr(hexv[:7], against), 2) if len(hexv) == 7 else None
        res.append(t)
    return res

D = lit(block(ds, 'D')); L = lit(block(ds, 'L'))
dark = tokens(D, '#23272c'); light = tokens(L, '#f2f4f6')

# ---- type, sizing ----
typ = lit(block(ds, 'type').split('].map')[0] + ']') if False else None
m = re.search(r'const type = (\[.*?\])\.map', ds, re.S)
type_rows = lit(m.group(1))
type_scale = [{'name': n, 'sample': s, 'size_px': fs, 'line_px': lh, 'weight': fw, 'family': 'IBM Plex Mono' if f == 'mono' else 'IBM Plex Sans',
               'tabular': f == 'mono'} for n, s, fs, lh, fw, f in type_rows]
m = re.search(r'const sizing = (\[.*?\])\.map', ds, re.S)
sizing = [{'name': n, 'value': v, 'note': u} for n, v, u in lit(m.group(1))]

# ---- clip palette (12 hues) ----
hues = lit(block(ds, 'hues')); names = lit(block(ds, 'hn'))
palette = []
for h, n in zip(hues, names):
    db, dw, lb, lw = ok2h(.35, .07, h), ok2h(.84, .1, h), ok2h(.88, .07, h), ok2h(.36, .1, h)
    palette.append({'name': n, 'hue': h, 'swatch': ok2h(.74, .13, h),
        'dark': {'body': db, 'header': ok2h(.46, .1, h), 'waveform': dw, 'border': ok2h(.55, .1, h), 'waveform_contrast': round(cr(dw, db), 2)},
        'light': {'body': lb, 'header': ok2h(.8, .1, h), 'waveform': lw, 'border': ok2h(.62, .1, h), 'waveform_contrast': round(cr(lw, lb), 2)}})

# ---- checks of the design's own stated minimums ----
problems = []
def check(theme, rows):
    for t in rows:
        if 'contrast' not in t or t['contrast'] is None: continue
        n = t['name']
        minimum = 4.5 if n in ('text.primary', 'text.secondary') or n == 'accent.on' else 3.0
        if n == 'text.disabled': continue  # exempt by design
        if t['contrast'] < minimum:
            problems.append(f"{theme} {n} {t['value']} on {t['contrast_against']}: {t['contrast']}:1 is under {minimum}:1")
check('dark', dark); check('light', light)
for p in palette:
    for th in ('dark', 'light'):
        if p[th]['waveform_contrast'] < 3: problems.append(f"palette {p['name']} {th}: waveform on body {p[th]['waveform_contrast']}:1 is under 3:1")

tokens_json = {
    'source': 'Claude Design bundle "Arrange Window Design Round 1" (14 October 2026), extracted as data; see docs/soundcraft-ai/design/ui-design-spec.md',
    'note': 'Colour contrast was recomputed with WCAG 2.x luminance. Palette values are computed from the design\'s OKLCH recipe. Names for the palette roles (body, header, waveform, border) are inferred from the design\'s variable names.',
    'fonts': [{'family': 'IBM Plex Sans', 'use': 'UI'}, {'family': 'IBM Plex Mono', 'use': 'numbers, counter, tabular'}],
    'themes': {'dark': {'default': True, 'surface_for_contrast': '#23272c', 'colors': dark}, 'light': {'default': False, 'surface_for_contrast': '#f2f4f6', 'colors': light}},
    'type_scale': type_scale, 'sizing': sizing, 'clip_palette': palette,
    'redlines': {
        'track_header': {'width': 232, 'height': 74, 'compact_height': 40, 'left_color_bar': 4, 'padding': 8, 'stack_heights': [6, 18, 4, 20, 4, 16, 6], 'note': 'vertical gaps and rows, top to bottom, sum to 74'},
        'clip': {'example_width': 330, 'height': 64, 'header': 14, 'fade_handle': 6, 'fade_zone': 14},
        'transport': {'button': [40, 40], 'play': [52, 40], 'counter': [316, 48], 'gap': 20},
    },
    'automation_modes': [{'name': n, 'glyph': g, 'meaning': l} for n, g, l in [('Off', '–', 'No automation is used'), ('Read', '▶', 'Plays the curve'), ('Write', '●', 'Overwrites the curve'), ('Touch', '◐', 'Writes while you hold a control'), ('Latch', '◉', 'Writes from first touch until stop')]],
    'checks': {'stated_minimums': 'text 4.5:1; accent, status and graphics 3:1; text.disabled exempt', 'problems': problems},
}
json.dump(tokens_json, open(f'{out}/tokens.json', 'w', encoding='utf-8'), indent=1, ensure_ascii=False)

# ---- notes and copy ----
notes_src = re.sub(r'\bN\(', '(', block(nc, 'notes'))
notes = [{'component': c, 'click_hover_keyboard': a, 'disabled_or_unavailable': d} for c, a, d in lit(notes_src)]
copy_src = block(nc, 'copy')
copy_src = re.sub(r'\bR\(', '(', copy_src)
copy_src = re.sub(r'([{,]\s*)(t|rows)\s*:', r"\1'\2':", copy_src)
copy = [{'section': g['t'], 'strings': [{'key': k, 'text': v} for k, v in g['rows']]} for g in lit(copy_src)]
json.dump({'note': 'Exact wording from the design (Notes and Copy). Words the design avoids: AI-powered, smart, magic, scores as percentages, wrong, correct, better. Strings with numbers are samples; fill them from real values at build time.', 'interaction_notes': notes, 'microcopy': copy}, open(f'{out}/copy.json', 'w', encoding='utf-8'), indent=1, ensure_ascii=False)

print(f"dark {len(dark)} light {len(light)} type {len(type_scale)} sizing {len(sizing)} palette {len(palette)} notes {len(notes)} copy groups {len(copy)} strings {sum(len(g['strings']) for g in copy)}")
print("contrast problems:", problems if problems else "none")
