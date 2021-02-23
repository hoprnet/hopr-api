import dayjs from 'dayjs'
import isSameOrAfter from 'dayjs/plugin/isSameOrAfter'

dayjs.extend(isSameOrAfter)

import { citiesMap } from '../constants';

export default async function handler(req, res) {
  const {
    query: { timestamp },
  } = req;

  const now = (timestamp ?
    process.env.NODE_ENV === 'production' ?
        new Date().getTime() :
        new Date(timestamp) : new Date()).getTime()

  const getCurrentCityFromTimestamp = (dateTimestamp) => {
    const citiesAfter = citiesMap.filter( city => dayjs(dateTimestamp).isSameOrAfter(city.date) )
    return citiesAfter.length > 0 ? citiesAfter.pop() : citiesMap[0]
  }

  const cityObject = getCurrentCityFromTimestamp(now)

  const city = cityObject.env
  const url = process.env[`VIMEO_URL_${city}`] || `https://vimeo.com/513096293`
  res.status(200).json({ url, city, now })
}
