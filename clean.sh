#!/bin/bash

node index.js $1 | tr '%' '"' | tr '\n' ','
