// Saldo e os 3 bônus do PokeIdle (XP, Captura, Shiny) de um canal da Twitch, rodando num player da Twitch
// da conta. Copiado do original (BulkRedeemDialog, leitura da Twitch).
(async a => {
 const base = { 'Client-Id': a.clientId, Authorization: 'OAuth ' + a.token, 'Content-Type': 'text/plain;charset=UTF-8',
  ...(a.device ? { 'X-Device-Id': a.device } : {}) };
 let integrity = null;
 const gql = async (query, variables) => {
  for (let i = 0; i < 2; i++) {
   const r = await fetch(a.api + '/gql', { method: 'POST', cache: 'no-cache',
    headers: integrity ? { ...base, 'Client-Integrity': integrity } : base, body: JSON.stringify({ query, variables }) });
   if (!r.ok) return { failed: 'erro ' + r.status };
   const j = await r.json();
   const challenged = !!j?.extensions?.challenge || (j?.errors || []).some(e => e?.extensions?.code === 'IntegrityCheckFailed');
   if (!challenged) return j;
   if (integrity) return { failed: 'checagem anti-robô da Twitch' };
   const t = await fetch(a.api + '/integrity', { method: 'POST', cache: 'no-cache', headers: base });
   const tj = t.ok ? await t.json() : null;
   if (!tj?.token) return { failed: 'checagem anti-robô da Twitch (' + t.status + ')' };
   integrity = tj.token;
  }
 };
 const fail = (j, x) => j?.failed || x?.error?.code || j?.errors?.[0]?.message || 'sem resposta';
 // O canal vende o mesmo bônus em pacotes (1h, 3h, 5h, 10h), todos pelo mesmo preço por hora: fica o maior
 // que o saldo paga; sem saldo para nenhum, o mais barato (o que falta menos).
 const pick = (all, points) => a.words.map(w => { const word = new RegExp(w, 'i');
  const list = all.filter(r => word.test(r.title) && /pokeidle/i.test(r.title + ' ' + (r.prompt || r.about || '')));
  const open = list.filter(r => !r.blocked), pool = open.length ? open : list;
  const fits = typeof points === 'number' ? pool.filter(r => r.cost <= points).sort((x, y) => y.cost - x.cost) : [];
  return fits[0] ?? pool.sort((x, y) => x.cost - y.cost)[0] ?? null; });
 try {
  const j = await gql('query($login:String!){currentUser{id login} user(login:$login){id channel{self{communityPoints{balance}}' +
   ' communityPointsSettings{customRewards{id title prompt cost isEnabled isPaused isInStock cooldownExpiresAt}}}}}', { login: a.channel });
  if (j?.failed) return { error: j.failed };
  const user = j?.data?.user;
  if (!user?.channel) return { error: 'canal não encontrado na Twitch' };
  const me = j?.data?.currentUser;
  const owner = !!me?.id && me.id === user.id;
  const points = user.channel.self?.communityPoints?.balance;
  if (!owner && typeof points !== 'number') return { error: 'sem o saldo na resposta (a conta entrou na Twitch?)' };
  const all = (user.channel.communityPointsSettings?.customRewards || []).filter(r => r.isEnabled).map(r => ({ id: r.id,
   title: r.title || '', prompt: r.prompt ?? null, cost: r.cost, blocked: r.isPaused ? 'pausado no canal'
    : !r.isInStock ? 'esgotado no canal' : r.cooldownExpiresAt ? 'em espera no canal' : null }));
  return { channelId: user.id, points: typeof points === 'number' ? points : null, owner, login: me?.login ?? null, rewards: pick(all, points) };
 } catch (e) { return { error: 'sem resposta da Twitch: ' + (e && e.message || e) }; }
})(__ARGS__).then(o => JSON.stringify(o))
