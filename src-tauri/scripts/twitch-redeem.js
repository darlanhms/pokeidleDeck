// Resgata um bônus de um canal da Twitch, rodando num player da Twitch da conta. Copiado do original
// (BulkRedeemDialog, resgate de um da Twitch).
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
 const pick = all => a.words.map(w => { const word = new RegExp(w, 'i');
  return all.find(r => word.test(r.title) && /pokeidle/i.test(r.title + ' ' + (r.prompt || r.about || ''))) ?? null; });
 try {
  const id = [...crypto.getRandomValues(new Uint8Array(16))].map(b => b.toString(16).padStart(2, '0')).join('');
  const r = await gql('mutation($i:RedeemCommunityPointsCustomRewardInput!){redeemCommunityPointsCustomReward(input:$i){redemption{id} error{code}}}',
   { i: { channelID: a.channelId, cost: a.cost, prompt: a.prompt, rewardID: a.rewardId, title: a.title, transactionID: id } });
  const p = r?.data?.redeemCommunityPointsCustomReward;
  if (p && !p.error && !r.errors) return { redeemed: true };
  return { error: fail(r, p) };
 } catch (e) { return { error: 'sem resposta da Twitch: ' + (e && e.message || e) }; }
})(__ARGS__).then(o => JSON.stringify(o))
