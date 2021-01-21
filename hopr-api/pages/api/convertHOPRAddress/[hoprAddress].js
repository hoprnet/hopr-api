const utils = require('@hoprnet/hopr-utils')
const channel = require('@hoprnet/hopr-core-ethereum')

const { hasB58String, convertPubKeyFromB58String, u8aToHex } = utils;
const { pubKeyToAccountId } = channel.Utils;

export default async function handler({ query: { hoprAddress } }, res) {
  if(!hasB58String(hoprAddress)) return res.status(200).json({ address: "invalid HOPR address"});
  const nativeAddress = String(u8aToHex(await pubKeyToAccountId((await convertPubKeyFromB58String(hoprAddress)).marshal()))).toLocaleLowerCase()
  res.status(200).json({ address: nativeAddress })
}
