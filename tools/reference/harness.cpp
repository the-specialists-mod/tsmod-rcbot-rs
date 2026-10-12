// Reference harness: runs RCBot's own A* (BotNavigate_AStarAlgo and the waypoint path functions,
// lifted verbatim from the upstream sources by build.sh) and its RCZ writer (rc_compress.cpp +
// miniz) on the real TS waypoint files, and prints what they produce. Engine and CBot pieces the
// code touches are stubbed below; the stubs answer as a TS server with an empty world would
// (every trace clear, no entities).
//
//   harness astar <file.rcw>     queries and results, one per line (see print_query)
//   harness rcz <n> <out.rcz>    the .rcv of a synthetic n-waypoint visibility table
//
// This file is GPL-3.0-or-later like the rest of the repository; the code build.sh pastes in is
// RCBot's (GPL-2.0-or-later with its exception).

#include <algorithm>
#include <cassert>
#include <cmath>
#include <cstdarg>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <iterator>
#include <new>

typedef float vec_t;
#include "vector.h"

// ---- engine and globals ----
struct entvars_t {
	int classname = 0;
	int spawnflags = 0;
	int solid = 0;
};
struct edict_t {
	entvars_t v;
};
struct TraceResult {
	float flFraction;
	edict_t* pHit;
};
enum IGNORE_MONSTERS { ignore_monsters = 1, dont_ignore_monsters = 0 };
enum IGNORE_GLASS { ignore_glass = 1, dont_ignore_glass = 0 };
constexpr int SOLID_TRIGGER = 1;
struct globalvars_t {
	float time = 0.0f;
};
static globalvars_t g_globals;
static globalvars_t* gpGlobals = &g_globals;
static const char* STRING(int) { return ""; }
static Vector AbsOrigin(const edict_t*) { return {0, 0, 0}; }
static void UTIL_TraceLine(const Vector&, const Vector&, IGNORE_MONSTERS, IGNORE_GLASS, edict_t*, TraceResult* tr) {
	tr->flFraction = 1.0f;
	tr->pHit = nullptr;
}
static edict_t* UTIL_FindEntityInSphere(edict_t*, const Vector&, float) { return nullptr; }
#define ALERT(...) ((void)0)
enum { at_error };

// RANDOM_FLOAT only times when failed goals are cleared; the tests don't compare it.
static float RANDOM_FLOAT(float lo, float) { return lo; }
static int RANDOM_LONG(int lo, int) { return lo; }
void BotMessage(edict_t*, int, const char*, ...) {}
static void DebugMessage(int, edict_t*, int, const char*, ...) {}

#include "generic_class.h"

constexpr int MAX_WAYPOINTS = 1024;
constexpr int MAX_PATH_INDEX = 4;
enum : std::uint8_t { FL_ASTAR_OPEN = 1 << 0, FL_ASTAR_CLOSED = 1 << 1, FL_ASTAR_PARENT = 1 << 2, FL_ASTAR_HEURISTIC = 1 << 3 };
enum { MOD_NS = 2, MOD_DMC = 7, MOD_TS = 8, MOD_GEARBOX = 9 };
enum { BOT_DEBUG_NAV_LEVEL = 1 };
enum { VALVE_WEAPON_HANDGRENADE = 12 };
enum class GearboxWeapon : std::uint8_t { GRAPPLE = 22 };

#include "upstream_decls.h"

struct CBotGlobals {
	bool IsMod(int m) const { return m == MOD_TS; }
	bool IsNS() const { return false; }
	bool IsDebugLevelOn(int) const { return false; }
};
static CBotGlobals gBotGlobals;

struct bot_profile_t {
	int m_iPathRevs = 150;
};

class CBot {
public:
	dataUnconstArray<AStarNode*> sOpenList;
	AStarNode aPathsFound[MAX_WAYPOINTS];
	dataUnconstArray<int> iDetPackWaypoints;
	bot_profile_t m_Profile;
	dataStack<int> m_FailedGoals;
	CBotFailedPaths m_stFailedPaths;
	float m_fNextClearFailedGoals = 0.0f;
	dataStack<int> m_stBotPaths;
	char m_szBotName[8] = "ref";
	Vector m_vGun;
	int m_iTeam = -1;

	float DistanceFrom(const Vector& v) const { return (v - m_vGun).Length(); }
	bool IsMarine() const { return false; }
	bool HasJetPack() const { return false; }
	bool IsFade() const { return false; }
	bool hasBlink() const { return false; }
	bool IsLerk() const { return false; }
	bool IsSkulk() const { return false; }
	bool IsOnos() const { return false; }
	bool HasWeapon(int) const { return false; }
	int GetTeam() const { return m_iTeam; }
};

class WAYPOINT {
public:
	int flags;
	Vector origin;
};
static WAYPOINT waypoints[MAX_WAYPOINTS];
static int num_waypoints;
static PATH* paths[MAX_WAYPOINTS];

void WaypointDebug() {}
bool WaypointFlagsOnLadderOrFly(int) { return false; }

#include "upstream_defs.inc"

// ---- the harness ----

static uint32_t g_seed = 12345;
static uint32_t lcg() {
	g_seed = g_seed * 1103515245u + 12345u;
	return g_seed >> 8;
}

static uint32_t bits(float f) {
	uint32_t u;
	std::memcpy(&u, &f, 4);
	return u;
}

// Reads the file the way WaypointLoad does for a valid RCBot file.
static bool load(const char* file) {
	std::FILE* fp = std::fopen(file, "rb");
	if (!fp)
		return false;
	unsigned char hdr[52];
	if (std::fread(hdr, 52, 1, fp) != 1)
		return false;
	int n;
	std::memcpy(&n, hdr + 16, 4);
	for (int i = 0; i < n; i++) {
		if (std::fread(&waypoints[i], 16, 1, fp) != 1)
			return false;
		num_waypoints++;
	}
	for (int i = 0; i < n; i++) {
		short num, p;
		if (std::fread(&num, 2, 1, fp) != 1)
			return false;
		for (int j = 0; j < num; j++) {
			if (std::fread(&p, 2, 1, fp) != 1)
				return false;
			WaypointAddPath(static_cast<short>(i), p);
		}
	}
	std::fclose(fp);
	return true;
}

static int astar(const char* file) {
	if (!load(file))
		return 1;
	int live[MAX_WAYPOINTS];
	int nlive = 0;
	for (int i = 0; i < num_waypoints; i++)
		if (!(waypoints[i].flags & (W_FL_DELETED | W_FL_AIMING | W_FL_UNREACHABLE)))
			live[nlive++] = i;
	if (nlive < 2)
		return 0;
	static const int teams[4] = {-1, -1, 0, 1};
	for (int q = 0; q < 120; q++) {
		CBot* bot = new CBot;
		const int from = live[lcg() % nlive];
		const int to = live[lcg() % nlive];
		bot->m_vGun = waypoints[live[lcg() % nlive]].origin + Vector(0, 0, 28.0f);
		bot->m_iTeam = teams[q % 4];
		bot->m_Profile.m_iPathRevs = (q % 3 == 2) ? 7 : 150;
		int frames = 1;
		int r = BotNavigate_AStarAlgo(bot, from, to, false);
		while (r == -2 && frames < 1000) {
			r = BotNavigate_AStarAlgo(bot, from, to, true);
			frames++;
		}
		// from to team gun.x gun.y gun.z revs -> result frames | path (stack top first)
		std::printf("%d %d %d %08x %08x %08x %d %d %d |", from, to, bot->m_iTeam, bits(bot->m_vGun.x), bits(bot->m_vGun.y),
			bits(bot->m_vGun.z), bot->m_Profile.m_iPathRevs, r, frames);
		while (!bot->m_stBotPaths.IsEmpty())
			std::printf(" %d", bot->m_stBotPaths.Pop());
		std::printf("\n");
		delete bot;
	}
	return 0;
}

#include "rc_compress.h"

static bool synthetic_visible(uint32_t i, uint32_t j) {
	uint32_t h = (i * 73856093u) ^ (j * 19349663u);
	h ^= h >> 13;
	h *= 0x5bd1e995u;
	h ^= h >> 15;
	return h % 3 == 0;
}

static int rcz(int n, const char* out) {
	static unsigned char table[MAX_WAYPOINTS * MAX_WAYPOINTS / 8];
	for (int i = 0; i < n; i++)
		for (int j = 0; j < n; j++)
			if (synthetic_visible(i, j)) {
				const int pos = i * MAX_WAYPOINTS + j;
				table[pos / 8] |= 1 << (pos % 8);
			}
	const uint32_t size = static_cast<uint32_t>((n * n + 7) / 8);
	return RCZ_CompressToFile(out, table, size) ? 0 : 1;
}

int main(int argc, char** argv) {
	if (argc == 3 && !std::strcmp(argv[1], "astar"))
		return astar(argv[2]);
	if (argc == 4 && !std::strcmp(argv[1], "rcz"))
		return rcz(std::atoi(argv[2]), argv[3]);
	std::fprintf(stderr, "usage: harness astar <file.rcw> | rcz <n> <out.rcz>\n");
	return 2;
}
