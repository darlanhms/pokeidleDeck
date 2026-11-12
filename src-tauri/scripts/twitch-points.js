// Saldo, baú e resgate automático da Twitch, rodando no player da live: o fetch da página leva a checagem
// anti-robô (Kasada) da Twitch. Copiado do original, PointsRedeemer.TwitchScript.
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
 try {
  const j = await gql('query($login:String!){user(login:$login){id channel{self{communityPoints{balance availableClaim{id}}}' +
   ' communityPointsSettings{customRewards{id title prompt cost isEnabled isPaused isInStock cooldownExpiresAt}}}}}', { login: a.channel });
  if (j?.failed) return { error: 'saldo: ' + j.failed };
  const user = j?.data?.user;
  if (!user?.channel) return { error: 'canal não encontrado' };
  const cp = user.channel.self?.communityPoints;
  if (typeof cp?.balance !== 'number') return { error: 'saldo: sem os pontos na resposta (login da Twitch?)' };
  let points = cp.balance; const out = { points };
  if (a.claim && cp.availableClaim?.id) {
   const k = await gql('mutation($i:ClaimCommunityPointsInput!){claimCommunityPoints(input:$i){claim{id} currentPoints error{code}}}',
    { i: { channelID: user.id, claimID: cp.availableClaim.id } });
   const p = k?.data?.claimCommunityPoints;
   if (p && !p.error && !k.errors) { out.claimed = true; points = typeof p.currentPoints === 'number' ? p.currentPoints : points + 50; }
   else out.claimError = fail(k, p);
   out.points = points;
  }
  if (!a.word) return out;
  const word = new RegExp(a.word, 'i');
  // Pacotes do mesmo bônus (1h a 10h): o maior que o saldo paga; nenhum cabe, o mais barato.
  const all = (user.channel.communityPointsSettings?.customRewards || []).filter(x => x.isEnabled && word.test(x.title || '')
   && /pokeidle/i.test((x.title || '') + ' ' + (x.prompt || ''))).sort((x, y) => x.cost - y.cost);
  const open = all.filter(x => !x.isPaused && x.isInStock !== false && !x.cooldownExpiresAt);
  const reward = open.filter(x => x.cost <= points).pop() ?? open[0] ?? all[0];
  if (!reward) return { ...out, error: 'canal sem o bônus' };
  Object.assign(out, { cost: reward.cost, title: reward.title, paused: reward.isPaused || !reward.isInStock || !!reward.cooldownExpiresAt });
  if (!a.spend || out.paused || points < reward.cost || (a.refund !== null && points >= a.refund)) return out;
  const id = [...crypto.getRandomValues(new Uint8Array(16))].map(b => b.toString(16).padStart(2, '0')).join('');
  const r = await gql('mutation($i:RedeemCommunityPointsCustomRewardInput!){redeemCommunityPointsCustomReward(input:$i){redemption{id} error{code}}}',
   { i: { channelID: user.id, cost: reward.cost, prompt: reward.prompt ?? null, rewardID: reward.id, title: reward.title, transactionID: id } });
  const p = r?.data?.redeemCommunityPointsCustomReward;
  if (p && !p.error && !r.errors) return { ...out, redeemed: true };
  return { ...out, refused: fail(r, p) };
 } catch (e) { return { error: 'sem resposta da Twitch: ' + (e && e.message || e) }; }
})(__ARGS__).then(o => JSON.stringify(o))
