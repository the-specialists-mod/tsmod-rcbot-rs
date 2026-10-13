// Reference harness for MegaHAL: runs RCBot's megahal.cpp (pasted in whole by build.sh, with
// BotFunc_FillString, strlow and UTIL_BuildFileName) on the fixture in
// crates/rcbot/tests/reference/hal/ and records brains and replies. Built 32-bit (-m32), as
// upstream ships, so `unsigned long` and `long` are 4 bytes.
//
//   hal_harness <fixture dir> <work dir> <out dir>
//
// RANDOM_LONG is rcbot::XorShiftRandom (random.rs), seeded with SEED, one sequence for the whole
// run. The CBot, client and config stubs answer what the fixture's chat lines say.
//
// This file is GPL-3.0-or-later like the rest of the repository; the code build.sh pastes in is
// RCBot's (GPL-2.0-or-later with its exception).

#include <cctype>
#include <cmath>
#include <cstdarg>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <new>
#include <string>
#include <vector>

// ---- the engine's RANDOM_LONG, as XorShiftRandom ----
static const uint64_t SEED = 20260613;
static uint64_t g_rng = SEED;
static uint64_t xorshift() {
	uint64_t x = g_rng;
	x ^= x << 13;
	x ^= x >> 7;
	x ^= x << 17;
	g_rng = x;
	return x;
}
static long RANDOM_LONG(long lo, long hi) {
	if (hi <= lo)
		return lo;
	const uint64_t span = static_cast<uint64_t>(static_cast<int64_t>(hi) - static_cast<int64_t>(lo) + 1);
	return static_cast<long>(static_cast<int64_t>(lo) + static_cast<int64_t>(xorshift() % span));
}

// ---- engine, globals and CBot stubs ----
struct entvars_t {
	const char* netname = "";
};
struct edict_t {
	entvars_t v;
};
static const char* STRING(const char* s) { return s; }
#define ALERT(...) ((void)0)
enum { at_console };
void BotMessage(edict_t*, int, const char*, ...) {}

constexpr int BOT_CHAT_MESSAGE_LENGTH = 128;
#define BOT_PROFILES_FOLDER "botprofiles"
constexpr int BOT_MAX_REP = 10;
constexpr int BOT_MIN_REP = 0;
enum { BOT_DEBUG_THINK_LEVEL = 1 };
enum {
	BOT_CONFIG_CHATTING = 1 << 2,
	BOT_CONFIG_REAL_MODE = 1 << 5,
	BOT_CONFIG_CHAT_DONT_LEARN = 1 << 8,
};

struct CClient {};
static CClient g_client;
static int g_sender_rep = 5;
static bool g_sender_is_bot = false;
static std::string g_bot_folder;

struct CClients {
	const CClient* GetClientByEdict(const edict_t*) const { return &g_client; }
};
class CBotGlobals {
public:
	int m_iConfigSettings = BOT_CONFIG_CHATTING | BOT_CONFIG_REAL_MODE;
	short m_iBotChatReplyPercent = 66;
	int m_iDebugLevels = 0;
	CClients m_Clients;
	bool IsConfigSettingOn(int f) const { return (m_iConfigSettings & f) == f; }
	const char* botFolder() const { return g_bot_folder.c_str(); }
};
CBotGlobals gBotGlobals;
char* g_argv = nullptr;

#include "hal_upstream_decls.h"

struct CBotReputations {
	int GetClientRep(const CClient*) const { return g_sender_rep; }
};
typedef struct bot_profile_s {
	int m_iProfileId = 0;
	char* m_szBotName = nullptr;
	char* m_szHAL_PreTrainFile = nullptr;
	char* m_szHAL_AuxFile = nullptr;
	char* m_szHAL_BanFile = nullptr;
	char* m_szHAL_SwapFile = nullptr;
	HAL_bot_t* m_HAL = nullptr;
	CBotReputations m_Rep;
} bot_profile_t;

class CBot {
public:
	bot_profile_t m_Profile;
	char* m_szBotName = nullptr;
	edict_t* m_pEdict = nullptr;
	edict_t* m_MegaHALTalkEdict = nullptr;
	bool IsUsed() const { return true; }
};
static CBot g_dummy_bot;
static CBot* UTIL_GetBotPointer(const edict_t*) { return g_sender_is_bot ? &g_dummy_bot : nullptr; }
static int UTIL_GetNumClients(bool) { return 3; }

#include "hal_upstream_defs.inc"

// ---- the harness ----

static std::vector<unsigned char> read_file(const std::string& path) {
	std::vector<unsigned char> v;
	std::FILE* fp = std::fopen(path.c_str(), "rb");
	if (!fp)
		return v;
	int c;
	while ((c = std::fgetc(fp)) != EOF)
		v.push_back(static_cast<unsigned char>(c));
	std::fclose(fp);
	return v;
}

static void write_file(const std::string& path, const std::vector<unsigned char>& v) {
	std::FILE* fp = std::fopen(path.c_str(), "wb");
	std::fwrite(v.data(), 1, v.size(), fp);
	std::fclose(fp);
}

static void copy(const std::string& from, const std::string& to) { write_file(to, read_file(from)); }

static std::vector<std::string> lines(const std::string& path) {
	std::vector<std::string> out;
	std::string cur;
	for (unsigned char c : read_file(path)) {
		if (c == '\n') {
			out.push_back(cur);
			cur.clear();
		} else
			cur += static_cast<char>(c);
	}
	return out;
}

static void hex(std::FILE* fp, const char* s) {
	std::fputc(' ', fp);
	if (!*s)
		std::fputc('-', fp);
	for (; *s; s++)
		std::fprintf(fp, "%02x", static_cast<unsigned char>(*s));
}

int main(int argc, char** argv) {
	if (argc != 4) {
		std::fprintf(stderr, "usage: hal_harness <fixture dir> <work dir> <out dir>\n");
		return 2;
	}
	const std::string fix = argv[1], work = argv[2], out = argv[3];
	g_bot_folder = work;
	// Where upstream looks for profile 1's lists (no separator after "botprofiles").
	copy(fix + "/hal.ban", work + "/botprofiles1_hal.ban");
	copy(fix + "/hal.aux", work + "/botprofiles1_hal.aux");
	copy(fix + "/hal.swp", work + "/botprofiles1_hal.swp");
	copy(fix + "/hal.trn", work + "/botprofiles1_hal.trn");

	// Profile 1: no brain yet, so an empty one is written, loaded and pretrained.
	static char name[] = "[RC]Neo";
	CBot bot;
	bot.m_szBotName = name;
	bot.m_Profile.m_iProfileId = 1;
	bot.m_Profile.m_szBotName = name;
	edict_t self;
	self.v.netname = name;
	bot.m_pEdict = &self;
	const bool pretrain = PrepareHALBrainForPersonality(&bot.m_Profile);
	if (!pretrain)
		return 1;
	copy(work + "/botprofiles/1_hal.brn", out + "/hal_empty.brn");
	LoadHALBrainForPersonality(&bot.m_Profile, true);
	SaveHALBrainForPersonality(&bot.m_Profile);
	copy(work + "/botprofiles/1_hal.brn", out + "/hal_trained.brn");

	// Chat: "sender<TAB>is_bot<TAB>rep<TAB>message". Per line: the message as the bot left it,
	// the reply ReplyToMessage keeps (127 bytes), and the reply after HumanizeString.
	std::FILE* rep = std::fopen((out + "/hal_replies.txt").c_str(), "w");
	for (const std::string& l : lines(fix + "/chat.txt")) {
		const size_t a = l.find('\t'), b = l.find('\t', a + 1), c = l.find('\t', b + 1);
		std::string sender_name = l.substr(0, a);
		g_sender_is_bot = l.substr(a + 1, b - a - 1) == "1";
		g_sender_rep = std::atoi(l.substr(b + 1, c - b - 1).c_str());
		edict_t sender;
		sender.v.netname = sender_name.c_str();
		char msg[512] = {};
		std::strcpy(msg, l.substr(c + 1).c_str());
		char reply[512] = {};
		BotChatReply(&bot, msg, &sender, reply);
		char said[BOT_CHAT_MESSAGE_LENGTH] = {};
		std::strncpy(said, reply, sizeof(said) - 1);
		std::fputs("R", rep);
		hex(rep, msg);
		hex(rep, said);
		if (said[0])
			HumanizeString(said);
		hex(rep, said);
		std::fputc('\n', rep);
	}
	std::fclose(rep);
	SaveHALBrainForPersonality(&bot.m_Profile);
	copy(work + "/botprofiles/1_hal.brn", out + "/hal_after_chat.brn");

	// Profile 2 wakes up with that brain (no lists, no training) and saves it again.
	copy(out + "/hal_after_chat.brn", work + "/botprofiles/2_hal.brn");
	bot_profile_t p2;
	p2.m_iProfileId = 2;
	p2.m_szBotName = name;
	if (PrepareHALBrainForPersonality(&p2))
		return 1;
	LoadHALBrainForPersonality(&p2, false);
	SaveHALBrainForPersonality(&p2);
	copy(work + "/botprofiles/2_hal.brn", out + "/hal_reloaded.brn");

	// RemoveNameTags on each name (in a zeroed buffer: its i + i reads land on zeroes).
	std::FILE* nm = std::fopen((out + "/hal_names.txt").c_str(), "w");
	for (const std::string& n : lines(fix + "/names.txt")) {
		char in[512] = {};
		char outname[512] = {};
		std::strcpy(in, n.c_str());
		RemoveNameTags(in, outname);
		std::fputs("N", nm);
		hex(nm, in);
		hex(nm, outname);
		std::fputc('\n', nm);
	}
	std::fclose(nm);
	return 0;
}
