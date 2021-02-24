import { utils, ethers } from 'ethers';


export default async function handler(req, res) {
  try {
    const {
      query: { secret },
    } = req;

    if (secret !== process.env.LBP_SCHEDULER_SECRET_KEY) {
      return res.status(401).json({ err: 'No secret passed' })
    }

    const BALANCER_LBP_BPT_HOPR_DAI_ADDRESS = "0x8cacF4C0F660EFDc3fd2e2266E86A9F57f794198"
    const BALANCER_LBP_BPT_ABI = [
      "function pokeWeights()"
    ];

    const provider = new ethers.providers.JsonRpcProvider(process.env.LBP_MAINNET_PROVIDER);
    const wallet = new ethers.Wallet(process.env.LBP_MAINNET_PRIVATE_KEY, provider);
    const balancerContract = new ethers.Contract(BALANCER_LBP_BPT_HOPR_DAI_ADDRESS, BALANCER_LBP_BPT_ABI, wallet);

    const response = await fetch("https://www.gasnow.org/api/v3/gas/price").then(res => res.json())
    const gasPrice = response.data.rapid;

    const overrides = {
      gasPrice,
      gasLimit: 100000
    };

    const tx = await balancerContract.pokeWeights(overrides);

    res.status(200).json({ tx })

  } catch(err) {
    if (err.code === 'INSUFFICIENT_FUNDS') {
      res.status(402).json({ err })
    } else {
      res.status(501).json({ err })
    }
  }
}