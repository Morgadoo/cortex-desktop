// Right-click on selected text: "Ask Lumo about the selection" keeps it for the popup to use.
chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({ id: 'ask-lumo', title: 'Ask Lumo about the selection', contexts: ['selection'] });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId !== 'ask-lumo') return;
  await chrome.storage.session.set({ selection: { text: info.selectionText || '', title: tab?.title || '', url: tab?.url || '' } });
  // Opens the popup where the browser allows it; otherwise the next click on the icon has it.
  try {
    await chrome.action.openPopup();
  } catch {
    /* the toolbar icon opens it */
  }
});
