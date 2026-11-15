"""Generates parity_fixture.json: synthetic accessibility-tree trajectories plus the region
units, effect rows, page inventories and episode paths that the Python reference model
(envmodel.py of the research prototype) derives from them.

Usage: python generate_parity_fixture.py PATH_TO_PROTOTYPE_DIR OUTPUT.json
Python's hash() is randomized per process, so only hash-independent quantities are recorded:
unit text, breadcrumb, occurrence counts, value-line counts, effect counters, inventories.
"""
import json, sys, collections
sys.path.insert(0, sys.argv[1])
import envmodel as em

def t(d, s): return '\t' * d + s

def nav(d, n=24):
    out = [t(d, "[1] navigation 'Main menu'")]
    for i in range(n):
        out.append(t(d + 1, f"[{10 + i}] link 'Section {i} of the portal'"))
        out.append(t(d + 2, f"StaticText 'Section {i} of the portal'"))
    return out

def page(title, body, bid0=200):
    lines = [f"RootWebArea '{title}'"]
    lines += nav(1)
    lines.append(t(1, "[5] main ''"))
    lines += [t(2, b) for b in body]
    return '\n'.join(lines)

def orders_body(n=14):
    out = ["[100] table 'Orders'", "\t[101] rowgroup ''"]
    for i in range(n):
        out.append(f"\t\t[{110 + i * 3}] row ''")
        out.append(f"\t\t\t[{111 + i * 3}] cell ''")
        out.append(f"\t\t\t\t[{112 + i * 3}] link 'Order #{1000 + i}'")
        out.append(f"\t\t\t\t\tStaticText 'Order #{1000 + i}'")
    return out

def detail_body(oid):
    return [f"[300] heading 'Order {oid}'", f"\tStaticText 'Order {oid}'", "[301] button 'Edit'", "[302] button 'Cancel order'",
            "[303] table 'Items'", "\t[304] row ''", "\t\t[305] cell ''", "\t\t\tStaticText 'Widget'"]

def form_body(val='', err=False, chk=False):
    out = ["[400] form 'Edit order'", f"\t[401] textbox 'Customer\\'s note' value='{val}' required",
           f"\t[402] checkbox 'Gift wrap'{' checked=true' if chk else ''}", "\t[403] button 'Save'", "\t[404] link 'Back to list'"]
    if err:
        out.append("\t[405] alert ''")
        out.append("\t\tStaticText 'Error: required field missing'")
    return out

H = 'http://shop.example.com'
def mk(i, oid, variant):
    S = []
    def st(url, title, body, action):
        S.append(dict(state_index=len(S), url=url, action=action, accessibility_tree=page(title, body)))
    st(f"{H}/admin/dashboard?session=ab12cd34ef", 'Dashboard - Admin', ["[50] heading 'Welcome back'", "\tStaticText 'Welcome back'"], None)
    st(f"{H}/admin/orders?page=1", 'Orders - Admin', orders_body(), "click('12')")
    st(f"{H}/admin/orders/{oid}", f'Order {oid} - Admin', detail_body(oid), f"click('{112 + 3 * (oid % 7)}')")
    st(f"{H}/admin/orders/{oid}/edit", f'Edit order {oid} - Admin', form_body(), "click('301')")
    if variant == 0:
        st(f"{H}/admin/orders/{oid}/edit", f'Edit order {oid} - Admin', form_body('gift for mum'), "fill('401', 'gift for mum')")
        st(f"{H}/admin/orders/{oid}/edit", f'Edit order {oid} - Admin', form_body('gift for mum', chk=True), "click('402')")
        st(f"{H}/admin/orders/{oid}", f'Order {oid} - Admin', detail_body(oid), "click('403')")
    elif variant == 1:
        st(f"{H}/admin/orders/{oid}/edit", f'Edit order {oid} - Admin', form_body(err=True), "click('403')")
        st(f"{H}/admin/orders/{oid}/edit", f'Edit order {oid} - Admin', form_body(err=True), "scroll(0, 400)")
        st(f"{H}/admin/orders?page=1", 'Orders - Admin', orders_body(), "click('404')")
        st(f"{H}/admin/dashboard?session=ff00ee11dd", 'Dashboard - Admin', ["[50] heading 'Welcome back'", "\tStaticText 'Welcome back'"], "goto('http://shop.example.com/admin/dashboard?session=ff00ee11dd')")
    else:
        st(f"{H}/admin/orders/{oid}/edit", f'Edit order {oid} - Admin', form_body(err=True), "click('403')")
        st(f"{H}/admin/orders/{oid}/edit", f'Edit order {oid} - Admin', form_body(err=True), "press('401', 'Enter')")
        st(f"{H}/admin/orders/{oid}/edit", f'Edit order {oid} - Admin', form_body('x'), "fill('401', 'x')")
    return dict(id=f"t{i}", goal=f"Add a gift note to order {oid}", outcome=['success', 'failure', 'success'][variant], states=S)

trajs = [mk(0, 1003, 0), mk(1, 1004, 1), mk(2, 1003, 2), mk(3, 1010, 0)]

m = em.Model()
for tr in trajs:
    m.add_traj(tr)

def tk(tm): return f"{tm[0]}||{tm[1]}"
units = sorted(({'text': u['text'], 'breadcrumb': u['bc'], 'occurrences': u['n'], 'episodes': len(u['traj']),
                 'values': dict(u['vals'])} for u in m.units.values() if u), key=lambda u: (u['text'], u['breadcrumb']))
effects = sorted(({'template': tk(k[0]), 'action': k[1], 'n': e['n'], 'loops': e['loop'], 'add': dict(e['add']), 'rem': dict(e['rem']),
                   'next': {tk(a): c for a, c in e['nxt'].items()}, 'err': dict(e['err'])} for k, e in m.eff.items()),
                key=lambda e: (e['template'], e['action']))
templates = sorted(({'template': tk(k), 'obs': v['n'], 'inventory': dict(v['inv'])} for k, v in m.tmpl.items()), key=lambda x: x['template'])
episodes = {k: v.split('\n', 1)[1] for k, v in m.eps.items()}
json.dump(dict(trajectories=trajs, units=units, effects=effects, templates=templates, episodes=episodes), open(sys.argv[2], 'w'), indent=1, sort_keys=True)
print(len(units), 'units', len(effects), 'effect rows', len(templates), 'templates')
