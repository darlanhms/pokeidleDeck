// Spike: a página do jogo manda ao deck, a cada segundo, o ouro (lido do recompra.js, que acompanha o
// socket do jogo) e se está na tela de login.
(() => {
  if (window.top !== window || window.__deckReport) return;
  Object.defineProperty(window, '__deckReport', { value: true });

  const loginScreen = () => {
    const p = document.querySelector('#portal'), l = document.querySelector('#login');
    return !!p && !!l && !p.classList.contains('hidden') && !l.classList.contains('hidden');
  };

  setInterval(() => {
    const invoke = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
    if (typeof invoke !== 'function') return;
    let s = null;
    try {
      s = window.__deckRecompra ? window.__deckRecompra.summary() : null;
    } catch { }
    const report = {
      gold: s && s.connected ? s.gold : null,
      nick: (s && s.nick) || null,
      connected: !!(s && s.connected),
      login: loginScreen(),
      url: location.href,
    };
    invoke('account_report', { report }).catch(() => { });
  }, 1000);
})();
