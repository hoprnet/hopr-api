export const citiesMap = [
  {
    date: 'Feb 24, 2021 15:59:59 UTC+09:00',
    destination: 'Unknown, ??',
    hour: '16:00 UTC+9',
    env: 'UNKNOWN',
    ht: 'Unknown',
    video: '',
  },
  {
    date: 'Feb 24, 2021 16:00:00 UTC+09:00',
    destination: 'Tokyo, JP',
    hour: '16:00 UTC+9',
    env: 'TOKYO',
    ht: 'Tokyo',
    video: '',
  },
  {
    date: 'Feb 24, 2021 17:00:00 UTC+09:00',
    destination: 'Seoul, KOR',
    hour: '17:00 UTC+9',
    env: 'SEOUL',
    ht: 'Seoul',
    video: '',
  },
  {
    date: 'Feb 24, 2021 17:00:00 UTC+08:00',
    destination: 'Shangai, CN',
    hour: '17:00 UTC+8',
    env: 'SHANGAI',
    ht: 'Shangai',
    video: '',
  },
  {
    date: 'Feb 24, 2021 17:00:00 UTC+07:00',
    destination: 'Hanoi, VN',
    hour: '17:00 UTC+7',
    env: 'HANOI',
    ht: 'Hanoi',
    video: '',
  },
  {
    date: 'Feb 24, 2021 14:00:00 UTC+03:00',
    destination: 'Moscow, RU',
    hour: '14:00 UTC+3',
    env: 'MOSCOW',
    ht: 'Moscow',
    video: '',
  },
  {
    date: 'Feb 24, 2021 15:00:00 UTC+03:00',
    destination: 'Istanbul, TR',
    hour: '15:00 UTC+3',
    env: 'ISTANBUL',
    ht: 'Istanbul',
    video: '',
  },
  {
    date: 'Feb 24, 2021 14:00:00 UTC+01:00',
    destination: 'Zurich, CH',
    hour: '14:00 UTC+1',
    env: 'ZURICH',
    ht: 'Zurich',
    video: '',
  },
  {
    date: 'Feb 24, 2021 15:00:00 UTC+01:00',
    destination: 'Madrid, ES',
    hour: '15:00 UTC+1',
    env: 'MADRID',
    ht: 'Madrid',
    video: '',
  },
  {
    date: 'Feb 24, 2021 12:00:00 UTC-03:00',
    destination: 'Sao Paulo, BR',
    hour: '12:00 UTC-3',
    env: 'SAO_PAULO',
    ht: 'SaoPaulo',
    video: '',
  },
  {
    date: 'Feb 24, 2021 8:00:00 UTC-09:00',
    destination: 'San Francisco, USA',
    hour: '8:00 UTC-9',
    env: 'SAN_FRANCISCO',
    ht: 'SF',
    video: '',
  }
]

export const citiesDict = citiesMap.reduce((accum, value) => Object.assign(accum, { [value.env]: value }), {})