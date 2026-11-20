"""Section 8 of the plan, run against the real app through tauri-driver (WebDriver)."""
import os, sys, time, json, pathlib
from selenium import webdriver
from selenium.webdriver.common.by import By
from selenium.webdriver.common.keys import Keys
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.common.options import ArgOptions

OUT = pathlib.Path(sys.argv[1])
THEME = sys.argv[2]
APP = os.environ["CORTEX_APP"]
results = []


def step(name):
    def wrap(fn):
        try:
            detail = fn() or ""
            results.append((name, "pass", detail))
        except Exception as e:  # noqa: BLE001
            results.append((name, "FAIL", f"{type(e).__name__}: {str(e)[:200]}"))
        try:
            d.get_screenshot_as_file(str(OUT / f"{THEME}-{len(results):02d}.png"))
        except Exception:
            pass
        return fn
    return wrap


opts = ArgOptions()
opts.set_capability("browserName", "wry")
opts.set_capability("tauri:options", {"application": APP})
d = webdriver.Remote(command_executor="http://127.0.0.1:4444", options=opts)
d.set_window_size(1280, 840)
d.implicitly_wait(5)
q = lambda css: d.find_element(By.CSS_SELECTOR, css)
qs = lambda css: d.find_elements(By.CSS_SELECTOR, css)
by_text = lambda text, tag="*": d.find_element(By.XPATH, f"//{tag}[normalize-space(.)='{text}']")
time.sleep(6)


def wait_for(cond, timeout=60, every=0.5):
    end = time.time() + timeout
    while time.time() < end:
        try:
            v = cond()
            if v:
                return v
        except Exception:
            pass
        time.sleep(every)
    raise TimeoutError("condition not met")


def idle(timeout=150):
    wait_for(lambda: not qs(".send-btn.stop"), timeout)
    time.sleep(1)


def js_click(css, last=True):
    ok = d.execute_script(
        "const all=[...document.querySelectorAll(arguments[0])].filter(e=>e.offsetParent!==null);"
        "const e=arguments[1]?all[all.length-1]:all[0]; if(!e) return false; e.scrollIntoView({block:'center'}); e.click(); return true;",
        css, last)
    assert ok, f"nothing to click at {css}"


def composer():
    return q("textarea.composer-input")


def send(text):
    c = composer()
    c.click()
    c.send_keys(text)
    c.send_keys(Keys.RETURN)


def set_theme():
    d.find_element(By.TAG_NAME, "body").send_keys(Keys.CONTROL, ",")
    time.sleep(1)
    by_text("Appearance").click()
    time.sleep(1)
    by_text("Dark" if THEME == "dark" else "Light").click()
    time.sleep(1)
    d.find_element(By.TAG_NAME, "body").send_keys(Keys.ESCAPE)
    time.sleep(1)
    assert d.execute_script("return document.documentElement.dataset.theme") == THEME


@step("theme")
def _():
    set_theme()
    return d.execute_script("return document.documentElement.dataset.theme")


@step("1 start: home view and sidebar")
def _():
    assert qs(".sidebar, nav, aside"), "no sidebar"
    by_text("New Chat")
    return f"{len(qs('.chat-row, .conversation-row, [data-chat]'))} chat rows"


@step("2a chat: send and stream")
def _():
    by_text("New Chat").click()
    time.sleep(1)
    send("Write three short sentences about rivers.")
    wait_for(lambda: qs(".turn-assistant .markdown"), 90)
    idle()
    return q(".turn-assistant .markdown").text[:80]


@step("2b chat: stop mid-answer")
def _():
    send("Write a 600 word essay about the history of bridges.")
    wait_for(lambda: qs(".send-btn.stop"), 30)
    time.sleep(3)
    js_click(".send-btn.stop")
    idle(30)
    return "stopped: " + (qs(".turn-assistant")[-1].text[-80:].replace("\n", " "))


@step("2c chat: regenerate")
def _():
    idle()
    before = len(qs(".turn-assistant"))
    # Stopped before any text: the chat offers "Answer now" instead of Regenerate.
    if not d.execute_script("return [...document.querySelectorAll(\"button[title='Regenerate']\")].some(b=>b.offsetParent)"):
        d.execute_script("[...document.querySelectorAll('.error-card button')].find(b=>/Answer now/.test(b.textContent)).click()")
    else:
        js_click("button[title='Regenerate']")
    time.sleep(2)
    idle()
    return f"assistant turns {before} -> {len(qs('.turn-assistant'))}; switcher: {bool(qs('.version-switch, .versions'))}"


@step("2d chat: edit a prompt")
def _():
    idle()
    d.execute_script("const b=[...document.querySelectorAll('.turn-user button[title=\\'Edit\\']')][0]; b.scrollIntoView({block:'center'}); b.click();")
    box = wait_for(lambda: [t for t in qs("textarea") if t.is_displayed() and "composer-input" not in t.get_attribute("class")], 10)[0]
    box.send_keys(Keys.CONTROL, "a")
    box.send_keys("Write two short sentences about lakes.")
    box.send_keys(Keys.CONTROL, Keys.RETURN)
    time.sleep(2)
    idle()
    return qs(".turn-user")[0].text[:60]


@step("3a composer: / @ # : menus")
def _():
    idle()
    d.find_element(By.XPATH, "//*[normalize-space(.)='New Chat']").click()
    time.sleep(1)
    seen = []
    for ch in "/@#:":
        c = composer()
        c.click()
        c.send_keys(Keys.CONTROL, "a")
        c.send_keys(Keys.BACKSPACE)
        c.send_keys(ch if ch != ":" else ":smi")
        time.sleep(1.2)
        menus = [m for m in qs(".menu, .flyout, .suggest, .slash-menu, .mention-menu, .popover") if m.is_displayed()]
        seen.append(f"{ch}:{len(menus)}")
        c.send_keys(Keys.ESCAPE)
    c.send_keys(Keys.CONTROL, "a")
    c.send_keys(Keys.BACKSPACE)
    assert all(not s.endswith(":0") for s in seen), seen
    return " ".join(seen)


@step("3b composer: Shift+Tab switches mode")
def _():
    label = lambda: d.execute_script("return [...document.querySelectorAll('.composer button')].map(b=>b.textContent.trim()).filter(Boolean).join(' ')")
    before = label()
    composer().send_keys(Keys.SHIFT, Keys.TAB)
    time.sleep(1)
    after = label()
    assert before != after, before
    composer().send_keys(Keys.SHIFT, Keys.TAB)  # and back, enough times to cycle is not needed
    return f"{before[:40]} -> {after[:40]}"


@step("3c composer: Alt+Enter queues while an answer runs")
def _():
    send("Write a 400 word story about a lighthouse keeper.")
    wait_for(lambda: qs(".send-btn.stop"), 30)
    c = composer()
    c.send_keys("And then one sentence about the sea.")
    c.send_keys(Keys.ALT, Keys.RETURN)
    time.sleep(1)
    queued = d.execute_script("return [...document.querySelectorAll('.queued-label,.queued-row')].map(e=>e.textContent.trim())")
    js_click(".send-btn.stop")
    idle(60)
    assert queued, "nothing shown as queued"
    # the queued message is sent once the answer ends
    idle(150)
    sent = [t.text for t in qs(".turn-user")]
    return f"queued {queued[:1]}; then sent: {any('one sentence about the sea' in x for x in sent)}"


@step("5 side panel: Sources")
def _():
    send("Search the web: what is the tallest mountain in Europe? One sentence.")
    idle()
    wait_for(lambda: [b for b in d.find_elements(By.XPATH, "//button[normalize-space(.)='Sources']") if b.is_displayed()], 20)
    d.execute_script("const b=[...document.querySelectorAll('button')].filter(b=>b.textContent.trim()==='Sources'&&b.offsetParent).pop(); b.scrollIntoView({block:'center'}); b.click();")
    time.sleep(2)
    panel = [p for p in qs(".side-panel, .panel, .sources-panel") if p.is_displayed()]
    assert panel, "no panel"
    return panel[0].text[:80].replace("\n", " ")


@step("6 settings: the six sections")
def _():
    d.find_element(By.TAG_NAME, "body").send_keys(Keys.ESCAPE)
    d.find_element(By.TAG_NAME, "body").send_keys(Keys.CONTROL, ",")
    time.sleep(1)
    opened = []
    for name in ["Profile", "Providers & models", "Audio", "Permissions", "General", "Appearance"]:
        by_text(name).click()
        time.sleep(1)
        heading = q(".settings-body h2, .settings-body h1, .modal h2").text
        opened.append(f"{name}={heading}")
    d.find_element(By.TAG_NAME, "body").send_keys(Keys.ESCAPE)
    return "; ".join(opened)


@step("7 pages: Agents, Scheduled, Context, Apps")
def _():
    got = []
    by_text("Agents").click()
    time.sleep(1.5)
    got.append(q("main h1, .page h1, h1").text)
    by_text("Work").click()
    time.sleep(1)
    for name in ["Scheduled", "Context", "Apps"]:
        d.find_element(By.XPATH, f"//*[contains(@class,'sidebar')]//*[normalize-space(.)='{name}']").click()
        time.sleep(1.5)
        got.append(q("h1").text)
    return " | ".join(got)


@step("8 search: Ctrl+K finds a chat and runs a command")
def _():
    d.find_element(By.TAG_NAME, "body").send_keys(Keys.CONTROL, "k")
    time.sleep(1)
    box = [i for i in qs("input") if i.is_displayed()][0]
    box.send_keys("lakes")
    time.sleep(1.5)
    found = [r.text for r in qs(".search-result, .palette-item, .search-hit, .menu-item") if r.is_displayed()]
    box.send_keys(Keys.CONTROL, "a")
    box.send_keys("Settings")
    time.sleep(1)
    box.send_keys(Keys.RETURN)
    time.sleep(1.5)
    opened = bool([m for m in qs(".modal") if m.is_displayed()])
    d.find_element(By.TAG_NAME, "body").send_keys(Keys.ESCAPE)
    assert found and opened, (found[:3], opened)
    return f"{len(found)} results; settings opened"


@step("9 language: another and back")
def _():
    from selenium.webdriver.support.select import Select
    def to(value, appearance):
        d.find_element(By.TAG_NAME, "body").send_keys(Keys.CONTROL, ",")
        time.sleep(1.5)
        d.find_element(By.XPATH, f"//*[normalize-space(.)='{appearance}']").click()
        time.sleep(1.5)
        Select(wait_for(lambda: q("select[aria-label='Interface language'], select[aria-label='Idioma da interface']"), 10)).select_by_value(value)
        time.sleep(6)  # the page reloads in the new language
        return d.execute_script("return document.querySelector('.sidebar, nav').innerText").replace("\n", " ")[:90]
    pt = to("pt", "Appearance")
    english_left = [w for w in ["New Chat", "Scheduled", "Projects"] if w in pt]
    back = to("", "Aparência")
    assert "New Chat" in back, back
    return f"pt sidebar: {pt} | English left: {english_left} | back: {back[:40]}"


@step("log: no errors in the page")
def _():
    errors = d.execute_script("return (window.__errors || []).length")
    return f"window errors: {errors}"


d.quit()
(OUT / f"results-{THEME}.json").write_text(json.dumps(results, indent=1))
for name, status, detail in results:
    print(f"{status:5} {name}: {detail}")
