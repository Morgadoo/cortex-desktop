"""Cortex adapts to its user: what they say about themselves in one chat is remembered, shown to
them, and used in the next chat without being said again."""
import json
import pathlib
import re
import sys
import time

from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.common.options import ArgOptions

OUT = pathlib.Path(sys.argv[1])
APP = str(pathlib.Path(__file__).resolve().parents[2] / "src-tauri/target/debug/cortex-desktop")
log = []

opts = ArgOptions()
opts.set_capability("browserName", "wry")
opts.set_capability("tauri:options", {"application": APP})
d = webdriver.Remote(command_executor="http://127.0.0.1:4444", options=opts)
d.set_window_size(1280, 840)
d.implicitly_wait(3)
time.sleep(6)
js = d.execute_script
qs = lambda css: d.find_elements(By.CSS_SELECTOR, css)


def note(name, ok, detail=""):
    log.append((name, "pass" if ok else "FAIL", detail))
    print(f"{'pass' if ok else 'FAIL':5} {name}: {detail}", flush=True)
    try:
        d.get_screenshot_as_file(str(OUT / f"adapt-{len(log):02d}.png"))
    except Exception:
        pass


def wait_for(cond, timeout=60):
    end = time.time() + timeout
    while time.time() < end:
        try:
            v = cond()
            if v:
                return v
        except Exception:
            pass
        time.sleep(0.5)
    return None


def new_chat():
    ok = js("""const el=[...document.querySelectorAll('.sidebar *')].find(e=>e.children.length<=2 && e.textContent.trim()==='New Chat' && e.closest('button,a,.nav-item,[role=button],.sidebar-item'));
               const b=el && (el.closest('button,a,.nav-item,[role=button],.sidebar-item')||el); if(!b) return false; b.click(); return true;""")
    assert ok, "no New Chat"
    time.sleep(1.2)


def ask(text):
    """Sends a message and waits for the answer to end, allowing any tool call it asks about."""
    c = d.find_element(By.CSS_SELECTOR, "textarea.composer-input")
    c.click()
    c.send_keys(text)
    c.send_keys(Keys.RETURN)
    wait_for(lambda: qs(".send-btn.stop"), 30)
    end = time.time() + 180
    while time.time() < end and qs(".send-btn.stop"):
        js("""const b=[...document.querySelectorAll('button')].find(b=>b.offsetParent && /^(Allow|Allow once|Approve)$/.test(b.textContent.trim())); if(b) b.click();""")
        time.sleep(1)
    time.sleep(1.5)
    return js("const m=[...document.querySelectorAll('.turn-assistant .markdown')].pop(); return m ? m.innerText : ''") or ""


def memories():
    return d.execute_async_script("""const done=arguments[arguments.length-1];
                 window.__TAURI__.core.invoke('chat_get_memories').then(m=>done(m.items.map(i=>i.text))).catch(e=>done(['error: '+e]));""")


MEAT = re.compile(r"\b(chicken|beef|pork|bacon|ham|lamb|steak|sausage|chorizo|fish|tuna|salmon|cod|bacalhau|shrimp|prawns?|meat)\b", re.I)

try:
    d.set_script_timeout(20)
    # One chat: the user says something about themselves.
    new_chat()
    said = ask("Please remember two things about me: I'm vegetarian, and I live in Porto. Reply in one short sentence.")
    saved = wait_for(lambda: [m for m in memories() if re.search(r"vegetarian|porto", m, re.I)], 20) or []
    note("what the user says about themselves is remembered", len(saved) >= 1, f"saved: {saved}; answer: {said[:100]!r}")

    # The user can see and change what is kept: it is listed as Facts on the Knowledge page.
    def click(text, scope=".sidebar"):
        return js("""const [want, scope]=arguments; const el=[...document.querySelectorAll(scope+' *')].find(e=>e.offsetParent && e.children.length<=2 && e.textContent.trim().replace(/\\s*\\d+$/, '')===want);
                     if(!el) return false; (el.closest('button,a,.nav-item,[role=button],[role=tab],.sidebar-item')||el).click(); return true;""", text, scope)
    click("Context")
    time.sleep(1)
    click("Knowledge")
    time.sleep(1.5)
    click("Facts", "main.main")
    # The fact itself, a row of the list (not the chat's title in the sidebar), brought into view.
    def fact(word):
        return js("""const want=arguments[0]; const el=[...document.querySelectorAll('main.main *')].find(e=>e.offsetParent && e.children.length===0 && e.textContent.toLowerCase().includes(want));
                     if(!el) return ''; el.scrollIntoView({block:'center'}); return el.textContent.trim();""", word)
    shown = wait_for(lambda: fact("porto") and fact("vegetarian"), 15)
    note("the user can see what was kept about them, as Facts on the Knowledge page", bool(shown), f"rows: {fact('vegetarian')!r}, {fact('porto')!r}")

    # A new chat, which has never been told: the answer follows what was learnt.
    new_chat()
    dinner = ask("Suggest one dinner I could cook tonight. One sentence, nothing else.")
    # "no meat", "meat-free" and the like say the opposite.
    plain = re.sub(r"\b(no|without|free of|instead of)\s+(\w+\s+)?(meat|fish)\b|\b(meat|fish)[- ](free|less)\b", "", dinner, flags=re.I)
    meat = MEAT.findall(plain)
    note("a new chat follows it without being told: dinner is vegetarian", bool(dinner) and not meat, f"{dinner[:160]!r}; meat words: {meat}")

    new_chat()
    place = ask("Name one market near where I live that I could visit this weekend. One sentence.")
    note("and knows where the user lives", "porto" in place.lower() or "bolhão" in place.lower() or "bolhao" in place.lower(), f"{place[:160]!r}")

    # It grows with the user: the three learning switches, turned on where the user would, on the
    # Knowledge page. They are off until then, since each costs model calls.
    def invoke(command, args=None):
        return d.execute_async_script("""const [c, a]=arguments; const done=arguments[arguments.length-1];
            window.__TAURI__.core.invoke(c, a||{}).then(done).catch(e=>done({error: String(e)}));""", command, args or {})

    # A second way of saying something already known, for the review to find.
    invoke("chat_add_memory", {"text": "Doesn't eat meat or fish."})
    # An agent that researches with web search: skills are learnt from an agent's work.
    invoke("chat_save_agent", {"agent": {"id": "price-scout", "name": "Price Scout", "tools": ["web_search"],
        "instructions": "You compare prices for the user. Use web search once per shop, then give the cheapest."}})
    js("location.reload()")
    time.sleep(6)
    click("Context")
    time.sleep(1)
    click("Knowledge")
    time.sleep(1.5)
    switched = []
    for title in ["Review my knowledge weekly", "Agents learn skills from their work", "Run my eval suites daily"]:
        ok = js("""const want=arguments[0]; const label=[...document.querySelectorAll('main.main *')].find(e=>e.children.length===0 && e.textContent.trim()===want);
                   if(!label) return false; let row=label; while(row && !row.querySelector('input.switch')) row=row.parentElement;
                   const s=row && row.querySelector('input.switch'); if(!s) return false; if(!s.checked) s.click(); return s.checked;""", title)
        switched.append((title, ok))
        time.sleep(0.8)
    learning = invoke("chat_learning")
    note("the user turns on learning on the Knowledge page", all(ok for _, ok in switched) and learning.get("reviewKnowledge") and learning.get("skillsFromWork") and learning.get("nightlyEvals"),
         f"{switched}; saved: review={learning.get('reviewKnowledge')} skills={learning.get('skillsFromWork')} evals={learning.get('nightlyEvals')}")

    # Skills from work: the agent does a procedure of several steps; a reflection after the turn may
    # keep it as a skill, turned off until the user reviews it.
    before = {s["name"] for s in invoke("chat_list_skills") or []}
    agent_row = js("""const r=[...document.querySelectorAll('.conv-link.project')].find(e=>e.textContent.includes('Price Scout')); if(!r) return false; r.click(); return true;""")
    time.sleep(1.5)
    work = ask("Every Monday I check the price of a 1 kg bag of basmati rice at Continente, Pingo Doce and Lidl in Portugal. Do this week's check: search each shop, then tell me the cheapest in two sentences.")
    steps = js("return document.querySelectorAll('.turn-assistant .tool-call, .turn-assistant .tool, .turn-assistant [data-tool]').length")
    learnt = wait_for(lambda: [s for s in invoke("chat_list_skills") or [] if s["name"] not in before], 120) or []
    notice = js("return [...document.querySelectorAll('.notice, .turn-notice, .chat-notice')].map(n=>n.innerText).join(' | ')") or ""
    note("an agent's repeated procedure becomes a skill, off until reviewed", bool(learnt) and all(not s.get("enabled", False) for s in learnt),
         f"agent chat opened={agent_row}; tool steps shown={steps}; learnt={[(s['name'], s.get('enabled')) for s in learnt]}; notice={notice[:160]!r}; answer={work[:100]!r}")

    # The weekly review and the daily evals run in the background, from the app's scheduler (every
    # 30 seconds): the review leaves its proposals in an unread chat, the evals a scored run.
    review = wait_for(lambda: [r for r in js("""return [...document.querySelectorAll('.conv-link[data-chat-id]')].map(r=>({title: r.querySelector('.conv-title')?.textContent||'', unread: !!r.querySelector('.unread-dot')}))""")
                               if r["unread"] and re.search(r"review|knowledge|memor|fact", r["title"], re.I)], 300) or []
    note("the weekly review leaves its proposals in an unread chat", bool(review), json.dumps(review)[:200])
    scored = wait_for(lambda: [s for s in invoke("chat_eval_suites") or [] if s.get("lastScore") is not None], 600) or []
    note("the daily evals score the starter suite on the user's model", bool(scored), json.dumps([(s["suite"], s["lastScore"], len(s["cases"])) for s in scored]))
    if review:
        js("""const r=[...document.querySelectorAll('.conv-link[data-chat-id]')].find(r=>r.querySelector('.unread-dot')); if(r) r.click();""")
        time.sleep(2)
        body = js("const m=[...document.querySelectorAll('.turn-assistant .markdown')].pop(); return m ? m.innerText : ''") or ""
        note("the review proposes merging the fact said twice", bool(re.search(r"duplicate|merge|same|overlap|redundant|combine", body, re.I)), body[:240].replace("\n", " | "))
finally:
    (OUT / "adapt.json").write_text(json.dumps(log, indent=1))
    d.quit()
