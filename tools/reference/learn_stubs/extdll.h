// Stub for the learning reference harness (tools/reference/build.sh): what ga.cpp, bot_ga.cpp,
// perceptron.cpp, bot_nn.cpp, som.cpp and bits.cpp need from the engine and the bot.
#pragma once
#include <algorithm>
#include <cassert>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <memory>
#include <numeric>
#include <vector>
struct edict_t;
int RANDOM_LONG(int lo, int hi);
float RANDOM_FLOAT(float lo, float hi);
void BotMessage(edict_t* pEntity, int iErr, const char* fmt, ...);
void UTIL_BuildFileName(char* filename, const char* arg1, const char* arg2);
#define BOT_PROFILES_FOLDER "profiles"
