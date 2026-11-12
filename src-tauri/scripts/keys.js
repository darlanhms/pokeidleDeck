// Atalhos que a página engoliria antes de chegarem ao deck.
(() => {
  if (window.top !== window) return;
  addEventListener('keydown', (e) => {
    if (e.key !== 'F11') return;
    const invoke = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
    if (typeof invoke !== 'function') return;
    e.preventDefault();
    invoke('page_key', { key: 'F11' }).catch(() => { });
  }, true);
})();
