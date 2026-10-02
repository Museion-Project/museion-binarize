"""Explicit-TOC source projection; measured observations remain immutable.

Printed references are source strings, independently of navigation scalars.
Discovery features are document independent and never use body-heading styles.
"""
import copy
import difflib
import re
import statistics
import unicodedata

from bookmarks import bbox, number, project, title_geometry, union

HEADINGS = ('contents', 'table of contents', 'inhalt', 'inhaltsverzeichnis',
            'inhaltsverzeichniss', 'sommaire', 'table des matieres', 'table des matieres detaillee',
            'indice', 'indice generale', 'sommario', 'contenido', 'contenidos',
            '目录', '目次', 'المحتويات')
AUXILIARY = ('list of figures', 'list of tables', 'list of plates', 'illustrations',
             'index locorum', 'index nominum', 'bibliography', 'bibliographie',
             'abbreviations', 'abkurzungen', 'references', 'index', 'indices',
             'index rerum', 'index verborum', 'general index', 'index of subjects',
             'index of names', 'index generalis')
REF = r'(?:[0-9]{1,5}|[ivxlcdmIVXLCDM]+)'
REFS = re.compile(rf'{REF}(?:\s*(?:[-–—−,;]|&)\s*{REF})*')


def printed_ref(text):
    text = text.strip()
    if not REFS.fullmatch(text):
        return None
    atoms = re.split(r'\s*(?:[-–—−,;]|&)\s*', text)
    return text if all(number(s) for s in atoms) else None


def key(text):
    return ' '.join(''.join(c for c in unicodedata.normalize('NFKD', text.casefold())
                           if not unicodedata.combining(c) and (c.isalnum() or c.isspace())).split())


def heading(text):
    value = key(text)
    if value in AUXILIARY:
        return False
    if value in HEADINGS:
        return True
    # Restrict short-word OCR tolerance to explicit character confusables;
    # edit-distance alone conflates semantic words (indices/indice, contentus).
    digit_normalized = value.translate(str.maketrans({'0':'o','1':'l','5':'s'}))
    if digit_normalized in HEADINGS:
        return True
    return len(value) >= 10 and len(value.split()) <= 5 and any(
        len(h) >= 10 and difflib.SequenceMatcher(None, value, h).ratio() >= .90
        for h in HEADINGS)


def observations(raw):
    rows = copy.deepcopy(raw['observations'])
    w, h = raw['width'], raw['height']
    for row in rows:
        for item in [row, *row.get('tokens', [])]:
            b = item['bbox']
            if raw['bbox_convention'] == 'normalized_bottom_left_xywh':
                b = [b[0]*w, (1-b[1]-b[3])*h, b[2]*w, b[3]*h]
            item['bbox'] = b
    return rows


def row_navigation(rows, width):
    """Pair visible numeric fragments with title rows before classifying roles."""
    med = statistics.median(r['bbox'][3] for r in rows) if rows else 12
    refs = {}
    for row in rows:
        words = row['text'].strip().split()
        for n in range(min(7, len(words)), 0, -1):
            ref = printed_ref(' '.join(words[-n:]))
            if ref:
                refs[row['id']] = (ref, n == len(words))
                break
    paired = set()
    for row in rows:
        value = refs.get(row['id'])
        if not value:
            continue
        if not value[1]:
            paired.add(row['id'])
            continue
        rb = row['bbox']; cy = rb[1]+rb[3]/2
        possible = [r for r in rows if r['id'] not in refs and
                    abs(r['bbox'][1]+r['bbox'][3]/2-cy) <= med*.8 and
                    (r['bbox'][0]+r['bbox'][2] <= rb[0]+med*.2 or
                     rb[0]+rb[2] <= r['bbox'][0]+med*.2)]
        if possible:
            nearest = min(possible, key=lambda r: abs(r['bbox'][1]+r['bbox'][3]/2-cy))
            paired.add(nearest['id']);paired.add(row['id'])
    return refs, paired, med


def features(raw):
    rows = sorted((r for r in observations(raw) if r['text'].strip() and
                   r['bbox'][1] < raw['height']*.94), key=lambda r:(r['bbox'][1],r['bbox'][0]))
    refs, paired, med = row_navigation(rows, raw['width'])
    heads = [r for r in rows if heading(r['text'])]
    # Bibliography WITH its printed reference is a TOC entry, not an auxiliary
    # page heading. Resolve row role before applying a lexical rejection.
    aux = [r for r in rows if key(r['text']) in AUXILIARY and
           r['id'] not in paired and r['bbox'][1] < raw['height']*.4]
    start = min((r['bbox'][1] for r in heads), default=0)
    scoped = [r for r in rows if r['bbox'][1] >= start]
    stop = raw['height']*.94
    paragraph = []
    for row in scoped:
        long = (len(row['text']) >= 70 and row['bbox'][2] >= raw['width']*.5 and
                row['id'] not in paired and not heading(row['text']))
        if long and (not paragraph or row['bbox'][1] - paragraph[-1]['bbox'][1] <= med*2.4):
            paragraph.append(row)
        else:
            paragraph = [row] if long else []
        if len(paragraph) == 4:
            stop = paragraph[0]['bbox'][1]
            break
    scoped = [r for r in scoped if r['bbox'][1] < stop]
    scoped_ids = {r['id'] for r in scoped}
    paired_count = sum(r['id'] in paired and not refs.get(r['id'],('',False))[1] for r in scoped)
    refs_count = sum(i in scoped_ids for i in refs)
    leaders = sum(bool(re.search(r'(?:[.·•_]\s*){3,}',r['text'])) for r in scoped)
    short_rows = sum(2 <= len(r['text'].split()) <= 18 and r['bbox'][2] < raw['width']*.85 for r in scoped)
    aux_header = bool(aux and not heads)
    nav = paired_count >= 2 and paired_count/max(1,len(scoped)) >= .12
    # Explicit headings can govern a genuinely unpaginated list, but never a
    # prose-only page. A paragraph starting immediately after the heading fails.
    unpaginated = len(scoped) >= 5 and short_rows >= 4 and short_rows/max(1,len(scoped)) >= .65
    seed = bool(heads) and not aux_header and (nav or unpaginated)
    return dict(seed=seed, continuation=bool(nav and not aux_header), heading=bool(heads),
                heading_text=[r['text'] for r in heads], auxiliary=[r['text'] for r in aux],
                reference_rows=refs_count, leader_rows=leaders, observed_rows=len(scoped),
                paired_navigation_rows=paired_count, region_y=[start/raw['height'],stop/raw['height']],
                paragraph_boundary=stop < raw['height']*.94,
                uncertainty=[] if seed else ['no_confirmed_explicit_navigation_seed'])


def spans(page_features):
    """Forward TOC spans: ordinary preceding pages never inherit a later seed."""
    groups = []
    active = False
    for i, f in enumerate(page_features):
        if f['seed'] or active and f['continuation']:
            if not active:groups.append([])
            groups[-1].append(i)
            active = not f.get('paragraph_boundary',False)
        else:
            active = False
    return groups


def restrict_region(raw, region):
    """Derived view only; raw observations remain on disk unchanged."""
    selected = copy.deepcopy(raw)
    point_rows = {r['id']:r for r in observations(raw)}
    lo,hi = region
    selected['observations'] = [r for r in selected['observations'] if
        lo <= (point_rows[r['id']]['bbox'][1]+point_rows[r['id']]['bbox'][3]/2)/raw['height'] <= hi]
    return selected


def clean_leaders(text):
    return re.sub(r'(?:[.·•_]\s*){2,}$', '', text).strip()


def visible_prefix(text):
    """Return a literal, visibly delimited label; never canonicalize numerals."""
    atom = r'(?:\d+|[IVXLCDMivxlcdm]+|[A-Za-zΑ-Ωα-ω])'
    match = re.match(rf'^({atom}(?:\.{atom})+[.)]?|{atom}[.)]|\d+)\s+(\S.*)$', text)
    return (match[1], match[2]) if match else (None, text)


def native_attached_reference(src, glyphs):
    """Split an inseparable leader/reference only at retained glyph boundaries."""
    text = src['text']
    match = re.search(rf'(?:[.·•_]\s*){{2,}}(?P<ref>{REF}(?:\s*[-–—−,;&]\s*{REF})*)\s*$', text)
    if not match or not printed_ref(match['ref']):
        return None
    gs = [glyphs[i] for i in src.get('glyph_ids', []) if i in glyphs]
    compact = ''.join(g['text'] for g in gs)
    wanted = re.sub(r'\s+', '', match['ref'])
    if not gs or not compact.endswith(wanted):
        return None
    offset = len(compact)-len(wanted);pos=0;ref_glyphs=[]
    for g in gs:
        if pos == offset or pos > offset:
            ref_glyphs.append(g)
        elif pos < offset < pos+len(g['text']):
            return None
        pos += len(g['text'])
    title = text[:match.start()].strip()
    title_glyphs = [g for g in gs if g['bbox'][0]+g['bbox'][2] <= ref_glyphs[0]['bbox'][0]] if ref_glyphs else []
    if not title or not title_glyphs:
        return None
    return dict(title=title, title_bbox=union([g['bbox'] for g in title_glyphs]),
                ref=dict(text=match['ref'],bbox=union([g['bbox'] for g in ref_glyphs]),
                         raw_id=src['id'],confidence=src.get('confidence')))


def source_projection(raw, raw_path, numeric_reads=None):
    base = project(raw, raw_path, numeric_reads)
    original = {r['id']: r for r in observations(raw)}
    rows = copy.deepcopy(base.get('visual_rows', []))
    w, h = raw['width'], raw['height']
    if not rows:
        return base, {}
    med = statistics.median(r['bbox'][3] for r in rows)
    glyphs = {g['id']:g for g in raw.get('glyphs', [])}
    for row in rows:
        src = original[row['id']]
        tokens = src.get('tokens', [])
        # Full inline references need no shared right edge (centered contents).
        # Require measured whitespace or a leader, not merely a terminal year.
        for n in range(min(5, len(tokens)-1), 0, -1):
            tail = ' '.join(t['text'] for t in tokens[-n:])
            if not printed_ref(tail):
                continue
            before = tokens[:-n]
            gap = tokens[-n]['bbox'][0] - (before[-1]['bbox'][0]+before[-1]['bbox'][2])
            if gap < med*.45 and not re.search(r'(?:[.·•_]\s*){2,}$', ' '.join(t['text'] for t in before)):
                continue
            row['page_token'] = dict(text=tail, bbox=union([t['bbox'] for t in tokens[-n:]]),
                                     raw_id=src['id'], confidence=src.get('confidence'))
            row['tokens'] = before
            row['text'] = ' '.join(t['text'] for t in before)
            row['bbox'] = union([t['bbox'] for t in before])
            break
        attached = native_attached_reference(src, glyphs)
        if attached:
            row['text'] = attached['title']
            row['bbox'] = attached['title_bbox']
            row['tokens'] = [dict(text=attached['title'],bbox=attached['title_bbox'])]
            row['page_token'] = attached['ref']
        row['text'] = clean_leaders(row['text'])
    # Attach separate full range tokens using the same measured row alignment.
    refs = [r for r in rows if printed_ref(r['text']) and not r.get('page_token')]
    consumed = set()
    for ref in refs:
        rb = ref['bbox']; cy = rb[1]+rb[3]/2
        candidates = [r for r in rows if r is not ref and not printed_ref(r['text'])
                      and r['bbox'][0]+r['bbox'][2] <= rb[0]+med*.2
                      and abs(r['bbox'][1]+r['bbox'][3]/2-cy) <= med*.65]
        if len(candidates) == 1 and not candidates[0].get('page_token'):
            candidates[0]['page_token'] = dict(text=ref['text'], bbox=rb, raw_id=ref['id'],
                                               confidence=ref.get('confidence'))
            consumed.add(ref['id'])
    rows = [r for r in rows if r['id'] not in consumed and r['text'] and
            not heading(r['text']) and not (printed_ref(r['text']) and
            (r['bbox'][1] < h*.13 or r['bbox'][1] > h*.91))]
    rows.sort(key=lambda r: (r.get('lane', 0), r['bbox'][1], r['bbox'][0]))
    groups = []
    for row in rows:
        join = False
        if groups:
            prev = groups[-1][-1]; b = row['bbox']; pb = prev['bbox']
            gap = b[1]-pb[1]-pb[3]
            numbered = visible_prefix(row['text'])[0] is not None
            # No scalar/range on the preceding logical entry: continuation may
            # complete its reference. Explicit numbered rows start a new entry.
            has_ref = any(r.get('page_token') for r in groups[-1])
            aligned = abs(b[0]-groups[-1][0]['bbox'][0]) < med*2.5
            label_only = number(prev['text'].rstrip('.)')) is not None
            join = (prev.get('lane') == row.get('lane') and -.25*med <= gap <= med*.95
                    and not numbered and not has_ref and
                    (aligned or label_only and abs((pb[0]+pb[2]/2)-(b[0]+b[2]/2)) < med*2))
            # An unnumbered all-capital container must not swallow its child.
            letters = ''.join(c for c in prev['text'] if c.isalpha())
            if len(letters) > 3 and letters.isupper() and not label_only:
                following = ''.join(c for c in row['text'] if c.isalpha())
                same_size = abs(prev.get('size',pb[3])-row.get('size',b[3])) <= med*.12
                same_axis = (abs(pb[0]-b[0]) <= med*.5 or
                             abs((pb[0]+pb[2]/2)-(b[0]+b[2]/2)) <= med*.5)
                join = join and len(following)>3 and following.isupper() and same_size and same_axis
        if join:
            groups[-1].append(row)
        else:
            groups.append([row])
    lines = []; full_refs = {}
    for i, group in enumerate(groups):
        ns = [r['page_token'] for r in group if r.get('page_token')]
        n = ns[0] if ns else None
        text = ' '.join(r['text'] for r in group)
        scalar = re.split(r'\s*[-–—−,;&]\s*', n['text'])[0] if n else None
        # The compiler's navigation scalar is an adapter field; full source
        # reference survives separately and is restored in canonical recovery.
        bounds = union([r['bbox'] for r in group]+([n['bbox']] if n else []))
        x, y, bw, bh = bounds
        clipped = [max(0,x), max(0,y), min(w,x+bw)-max(0,x), min(h,y+bh)-max(0,y)]
        if clipped[2] <= 0 or clipped[3] <= 0:
            base.setdefault('diagnostics', []).append('off_page_observation_rejected')
            continue
        tx, _ = title_geometry(group[0])
        ident = f'p{raw["page_index"]}-r{i}'
        line = dict(id=ident, group_id=ident, raw_ids=list(dict.fromkeys([r['id'] for r in group]+[n['raw_id']] if n else [r['id'] for r in group])),
                    text=text+(' '+scalar if scalar else ''), bbox=bbox(clipped),
                    title_start_x=tx if tx is not None and clipped[0] <= tx <= clipped[0]+clipped[2] else None,
                    size_proxy=group[0].get('size',group[0]['bbox'][3]), confidence=group[0].get('confidence'),
                    source_kind=raw.get('source_kind','apple_vision_fast'),
                    state='ambiguous' if bounds != clipped or any(r.get('ambiguous') for r in group) else raw.get('state','locally_recognized'),
                    printed_candidate=scalar)
        lines.append(line)
        full_refs[ident] = n['text'] if n else None
    base['lines'] = lines
    base['projection_version'] = 'explicit-toc-source/1'
    return base, full_refs
