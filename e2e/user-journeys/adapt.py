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
finally:
    (OUT / "adapt.json").write_text(json.dumps(log, indent=1))
    d.quit()
