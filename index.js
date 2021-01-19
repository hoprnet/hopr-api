const util = require('util')
const readFile = util.promisify(require('fs').readFile)

const main = async() => {
  try {
    const file = await readFile('tripetto_form.csv')
    console.log('File', file)
  } catch(e) {
    console.error('Error', e)
  }
}

main()
