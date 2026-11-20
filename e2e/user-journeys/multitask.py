"""Real-user multitasking against the real app: switch chats while answers run, keep drafts, see
progress, and come back after a restart."""
import os, sys, time, json, pathlib
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.common.options import ArgOptions

OUT = pathlib.Path(sys.argv[1])
PHASE = sys.argv[2]  # "run" or "after-restart"
APP = os.environ["CORTEX_APP"]
log = []


def note(name, ok, detail=""):
    log.append((name, "pass" if ok else "FAIL", detail))
    print(f"{'pass' if ok else 'FAIL':5} {name}: {detail}", flush=True)
    try:
        d.get_screenshot_as_file(str(OUT / f"mt-{PHASE}-{len(log):02d}.png"))
    except Exception:
        pass


opts = ArgOptions()
opts.set_capability("browserName", "wry")
opts.set_capability("tauri:options", {"application": APP})
d = webdriver.Remote(command_executor="http://127.0.0.1:4444", options=opts)
d.set_window_size(1280, 840)
d.implicitly_wait(3)
time.sleep(6)
js = d.execute_script
qs = lambda css: d.find_elements(By.CSS_SELECTOR, css)


def wait_for(cond, timeout=60):
    end = time.time() + timeout
    while time.time() < end:
        try:
            v = cond()
            if v:
                return v
        except Exception:
            pass
        time.sleep(0.4)
    return None


def new_chat():
    ok = js("""const el=[...document.querySelectorAll('.sidebar *')].find(e=>e.children.length<=2 && e.textContent.trim()==='New Chat' && e.closest('button,a,.nav-item,[role=button],.sidebar-item'));
               const b=el && (el.closest('button,a,.nav-item,[role=button],.sidebar-item')||el); if(!b) return false; b.click(); return true;""")
    assert ok, "no New Chat"
    time.sleep(1.2)


def type_in_composer(text, send=True):
    c = d.find_element(By.CSS_SELECTOR, "textarea.composer-input")
    c.click()
    c.send_keys(text)
    if send:
        c.send_keys(Keys.RETURN)


def answer_text():
    return js("const t=[...document.querySelectorAll('.turn-assistant')].pop(); return t ? t.innerText : ''") or ""


def answer_md():
    """The answer's own text, without the thinking panel (which collapses once the answer starts)."""
    return js("const m=[...document.querySelectorAll('.turn-assistant .markdown')].pop(); return m ? m.innerText.length : 0") or 0


def rows():
    return js("""return [...document.querySelectorAll('.conv-link')].map(r => ({
        id: r.dataset.chatId, title: r.querySelector('.conv-title')?.textContent,
        running: !!r.querySelector('.dots'), unread: !!r.querySelector('.unread-dot'),
        active: r.classList.contains('active')}))""")


def open_chat(cid):
    js("document.querySelector(`.conv-link[data-chat-id='${arguments[0]}']`).click()", cid)
    time.sleep(1.5)


def current_id():
    return js("return document.querySelector('.conv-link.active')?.dataset.chatId")


import atexit
atexit.register(lambda: d.quit())
if PHASE == "run":
    # A: a long answer starts.
    new_chat()
    type_in_composer("Answer directly in the chat, without tools or canvases: a 1500 word story about a fox who learns to sail, in many paragraphs. Plain prose, no headings.")
    started = wait_for(lambda: answer_md() > 40, 180)
    a_id = wait_for(current_id, 10)
    note("A streams", bool(started), f"{answer_md()} chars of answer so far")

    # Leave A while it runs: a new chat, with a draft typed and not sent.
    new_chat()
    type_in_composer("half-written thought about tides", send=False)
    time.sleep(1)
    r = {x["id"]: x for x in rows()}
    note("A shows as running in the sidebar while away", r.get(a_id, {}).get("running", False), json.dumps(r.get(a_id)))

    # Go back to A: the text so far is there and keeps growing.
    open_chat(a_id)
    first = answer_md()
    time.sleep(4)
    second = answer_md()
    note("back in A, the answer so far is shown and keeps streaming", second > first, f"{first} -> {second} chars")

    # B: the draft kept? (B was never sent, so it is the New chat page again.)
    new_chat()
    draft = js("return document.querySelector('textarea.composer-input').value")
    note("an unsent draft on the new-chat page survives switching away", draft == "half-written thought about tides", repr(draft))
    c = d.find_element(By.CSS_SELECTOR, "textarea.composer-input")
    c.send_keys(Keys.CONTROL, "a")
    c.send_keys(Keys.BACKSPACE)

    # B: a second answer runs at the same time as A.
    type_in_composer("List five fruits, one per line, nothing else.")
    b_started = wait_for(lambda: current_id() and current_id() != a_id, 20)
    b_id = current_id()
    both = wait_for(lambda: (lambda r: r if len(r) >= 2 else None)([x for x in rows() if x["running"]]), 20)
    titles = [x["title"] for x in (both or [])]
    note("two answers run at once, both marked in the sidebar and told apart by title",
         bool(both) and len(set(titles)) == len(titles) and "New chat" not in titles, json.dumps(titles))

    # Wait for B; stay on B until A finishes in the background.
    wait_for(lambda: not qs(".send-btn.stop"), 120)
    a_done = wait_for(lambda: not {x["id"]: x for x in rows()}.get(a_id, {}).get("running", True), 240)
    r = {x["id"]: x for x in rows()}
    note("A finishes in the background and is marked unread", bool(a_done) and r.get(a_id, {}).get("unread", False), json.dumps(r.get(a_id)))

    # Open A: complete answer, unread cleared.
    open_chat(a_id)
    text = answer_text()
    words = len(text.split())
    r = {x["id"]: x for x in rows()}
    # The model writes the title just after the answer ends, in the background.
    title_of_a = lambda: {x["id"]: x for x in rows()}.get(a_id, {}).get("title", "")
    final_title = wait_for(lambda: (lambda t: t if t and not t.startswith("Answer directly") else None)(title_of_a()), 30) or title_of_a()
    note("A's provisional title is replaced by the model's", bool(final_title) and not final_title.startswith("Answer directly"), final_title)
    note("A holds the whole answer, and opening it clears unread", words > 700 and not r.get(a_id, {}).get("unread", True), f"{words} words")

    # C: a long answer that the app is closed in the middle of.
    new_chat()
    type_in_composer("Answer directly in the chat, without tools or canvases: a 2500 word essay about lighthouses, in many paragraphs. Plain prose.")
    md = lambda: js("const m=[...document.querySelectorAll('.turn-assistant .markdown')].pop(); return m ? m.innerText.length : 0") or 0
    wait_for(lambda: md() > 150, 180)
    c_id = current_id()
    (OUT / "mt-ids.json").write_text(json.dumps({"a": a_id, "b": b_id, "c": c_id, "partial": md()}))
    note("C streaming when the app is closed", True, f"{len(answer_text())} chars shown")
    # A crash, not a close to the tray: the process dies mid-answer.
    import subprocess
    subprocess.run(["pkill", "-9", "-f", APP])
    time.sleep(1)
else:
    ids = json.loads((OUT / "mt-ids.json").read_text())
    r = {x["id"]: x for x in rows()}
    note("after restart, the chats are all listed", all(ids[k] in r for k in "abc"), json.dumps([r.get(ids[k], {}).get("title") for k in "abc"]))
    open_chat(ids["c"])
    text = answer_text()
    body = js("return document.querySelector('.chat, main').innerText")
    note("after restart, C is marked as cut off, with what it had done", (("closed before this answer finished" in body and len(text) > 20) or "stopped before it finished" in body or (ids["partial"] == 0 and "No answer to this message" in body)),
         f"answer {len(text)} chars (had {ids['partial']}); tail: {body[-160:]!r}")
    # Continue: the cut-off answer goes on to the end.
    ok = js("""const b=[...document.querySelectorAll('.error-card button')].find(b=>/Continue|Answer now/.test(b.textContent)); if(!b) return false; b.click(); return true;""")
    started = wait_for(lambda: qs(".send-btn.stop"), 30)
    wait_for(lambda: not qs(".send-btn.stop"), 240)
    time.sleep(2)
    final = js("return [...document.querySelectorAll('.turn-assistant')].map(t=>t.innerText).join('\\n')")
    note("Continue finishes the cut-off answer", bool(ok) and bool(started) and len(final.split()) > 300 and not qs(".error-card"),
         f"clicked={ok}; {len(final.split())} words across the answer")

(OUT / f"mt-{PHASE}.json").write_text(json.dumps(log, indent=1))
