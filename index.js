const util = require('util')
const csv = require('fast-csv')
const fs = require('fs')
const utils = require('@hoprnet/hopr-utils')
const channel = require('@hoprnet/hopr-core-ethereum')

const { hasB58String, convertPubKeyFromB58String, u8aToHex } = utils;
const { pubKeyToAccountId } = channel.Utils;

const main = async() => {
  const addresses = []
  try {
    fs.createReadStream(process.argv[2])
    .pipe(csv.parse({ headers: true, delimiter: ';' }))
    // pipe the parsed input into a csv formatter
    .pipe(csv.format({ headers: false }))
    // Using the transform function from the formatting stream
    .transform(async (row, next) => {
      // const hoprAddressColumn = 'HOPR ADDR'
      // const nativeAddressColumn = 'BNB ADDR'
      const hoprAddressColumn = 'F5' // column headers change sometimes
      const nativeAddressColumn = 'F3' // column headers change sometimes
      if(!row[hoprAddressColumn] || !row[nativeAddressColumn]) return next(false);
      if(!hasB58String(row[hoprAddressColumn])) return next(false);
      const hoprAddress = row[hoprAddressColumn];
      const maybeNativeAddress = String(row[nativeAddressColumn]).toLocaleLowerCase()
      const nativeAddress = String(u8aToHex(await pubKeyToAccountId((await convertPubKeyFromB58String(hoprAddress)).marshal()))).toLocaleLowerCase()
      if(maybeNativeAddress != nativeAddress) return next(false);
      //addresses.push(nativeAddress);
      return next(null, {
        address: `%${nativeAddress}%`
      });
    })
    .pipe(process.stdout)
    .on('end', () => {
      process.exit()
    });
  } catch(e) {
    console.error('Error', e)
  }
}

main()
