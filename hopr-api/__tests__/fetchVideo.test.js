import { createMocks } from 'node-mocks-http';
import fetchVideo from '../pages/api/fetchVideo';
import { citiesMap, citiesDict } from '../utils/constants';

describe('/api/fetchVideo', () => {
  test('Starts in TOKYO timezone', async () => {
    const { req, res } = createMocks({
      method: 'GET',
    });
    await fetchVideo(req, res);
    expect(res._getStatusCode()).toBe(200);
    expect(JSON.parse(res._getData())).toEqual(
      expect.objectContaining({
        city: 'TOKYO',
      }),
    );
  });

  test('HANOI timezone', async () => {
    const { req, res } = createMocks({
      method: 'GET',
      query: {
        timestamp: new Date(citiesDict['HANOI'].date).getTime(),
      }
    });
    await fetchVideo(req, res);
    expect(res._getStatusCode()).toBe(200);
    expect(JSON.parse(res._getData())).toEqual(
      expect.objectContaining({
        city: 'HANOI',
      }),
    );
  });

  test('MADRID - 1 should be ZURICH', async () => {
    const { req, res } = createMocks({
      method: 'GET',
      query: {
        timestamp: (new Date(citiesDict['MADRID'].date).getTime()) - 1,
      }
    });
    await fetchVideo(req, res);
    expect(res._getStatusCode()).toBe(200);
    expect(JSON.parse(res._getData())).toEqual(
      expect.objectContaining({
        city: 'ZURICH',
      }),
    );
  });

  test('ZURICH + 1 should be ZURICH', async () => {
    const { req, res } = createMocks({
      method: 'GET',
      query: {
        timestamp: (new Date(citiesDict['ZURICH'].date).getTime()) + 1,
      }
    });
    await fetchVideo(req, res);
    expect(res._getStatusCode()).toBe(200);
    expect(JSON.parse(res._getData())).toEqual(
      expect.objectContaining({
        city: 'ZURICH',
      }),
    );
  });

  test('ZURICH + 1hr should be MADRID', async () => {
    const { req, res } = createMocks({
      method: 'GET',
      query: {
        timestamp: (new Date(citiesDict['ZURICH'].date).getTime()) + 1*60*60*1000,
      }
    });
    await fetchVideo(req, res);
    expect(res._getStatusCode()).toBe(200);
    expect(JSON.parse(res._getData())).toEqual(
      expect.objectContaining({
        city: 'MADRID',
      }),
    );
  });

  test('ZURICH - 1hr should be ISTANBUL', async () => {
    const { req, res } = createMocks({
      method: 'GET',
      query: {
        timestamp: (new Date(citiesDict['ZURICH'].date).getTime()) - 1*60*60*1000,
      }
    });
    await fetchVideo(req, res);
    expect(res._getStatusCode()).toBe(200);
    expect(JSON.parse(res._getData())).toEqual(
      expect.objectContaining({
        city: 'ISTANBUL',
      }),
    );
  });

    test('ZURICH - 1hr - 1 should be MOSCOW', async () => {
    const { req, res } = createMocks({
      method: 'GET',
      query: {
        timestamp: (new Date(citiesDict['ZURICH'].date).getTime()) - 1*60*60*1000 - 1,
      }
    });
    await fetchVideo(req, res);
    expect(res._getStatusCode()).toBe(200);
    expect(JSON.parse(res._getData())).toEqual(
      expect.objectContaining({
        city: 'MOSCOW',
      }),
    );
  });

  test('Ends in SAN_FRANCISCO', async () => {
    const { req, res } = createMocks({
      method: 'GET',
      query: {
        timestamp: new Date("Mar 1, 2021 12:00:00 UTC-09:00").getTime(),
      }
    });
    await fetchVideo(req, res);
    expect(res._getStatusCode()).toBe(200);
    expect(JSON.parse(res._getData())).toEqual(
      expect.objectContaining({
        city: 'SAN_FRANCISCO',
      }),
    );
  });


});