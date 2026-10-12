// Learning reference harness: runs RCBot's own GA, individuals, perceptron, NN, SOM and bit
// arrays (ga.cpp, bot_ga.cpp, perceptron.cpp, bot_nn.cpp, som.cpp, bits.cpp, compiled unchanged
// by build.sh against the stub headers in learn_stubs/) on fixed scenarios, and writes what they
// save. RANDOM_LONG / RANDOM_FLOAT are the same xorshift generator as the port's XorShiftRandom,
// so crates/rcbot/tests/upstream_learn.rs can replay each scenario and compare the bytes.
//
//   learn_harness <out dir>
//
// This file is GPL-3.0-or-later like the rest of the repository; the code it links is RCBot's
// (GPL-2.0-or-later with its exception).

#include "extdll.h"
#include "bot.h"
#include "bits.h"
#include "nn.h"
#include "som.h"

#include <string>

// ---- XorShiftRandom (crates/rcbot/src/random.rs), bit for bit ----
static uint64_t g_state = 1;

static void seed(uint64_t s) { g_state = s ? s : 0x9E3779B97F4A7C15ull; }

static uint64_t next_u64() {
	uint64_t x = g_state;
	x ^= x << 13;
	x ^= x >> 7;
	x ^= x << 17;
	g_state = x;
	return x;
}

int RANDOM_LONG(int lo, int hi) {
	if (hi <= lo)
		return lo;
	const uint64_t span = static_cast<uint64_t>(static_cast<int64_t>(hi) - static_cast<int64_t>(lo) + 1);
	return static_cast<int>(static_cast<int64_t>(lo) + static_cast<int64_t>(next_u64() % span));
}

float RANDOM_FLOAT(float lo, float hi) {
	const float unit = static_cast<float>(next_u64() >> 40) / static_cast<float>(1ull << 24);
	return lo + (hi - lo) * unit;
}

void BotMessage(edict_t*, int, const char*, ...) {}
void UTIL_BuildFileName(char* out, const char*, const char*) { out[0] = 0; }

// ---- output ----
static std::string g_dir;

static std::FILE* out_file(const char* name) {
	const std::string path = g_dir + "/" + name;
	std::FILE* f = std::fopen(path.c_str(), "wb");
	if (!f) {
		std::perror(path.c_str());
		std::exit(1);
	}
	return f;
}

static void put_f32(std::FILE* f, float v) { std::fwrite(&v, sizeof v, 1, f); }
static void put_u32(std::FILE* f, unsigned v) { std::fwrite(&v, sizeof v, 1, f); }

// ---- scenarios (mirrored in tests/upstream_learn.rs) ----

// A CGA(6) of 5-gene CBotGAValues: 40 scored lives; every third life a pick (whose genes are
// written out) when the GA has one. Then the GA as CGA::save writes it, and the same bytes loaded
// back by upstream and saved again.
static void ga_scenario() {
	seed(101);
	CGA ga(6);
	std::FILE* f = out_file("learn_ga.bin");
	for (int life = 0; life < 40; life++) {
		CBotGAValues* v = new CBotGAValues();
		for (int i = 0; i < 5; i++)
			v->add(RANDOM_FLOAT(-1, 1));
		v->setFitness(RANDOM_FLOAT(0, 2));
		ga.addToPopulation(v);
		if (life % 3 == 2 && ga.canPick()) {
			CBotGAValues* p = static_cast<CBotGAValues*>(ga.pick());
			p->save(f);
			delete p;
		}
	}
	ga.save(f);
	put_u32(f, ga.getNumGenerations());
	std::fclose(f);
}

// A CGA(5) of CIntGAValues, adaptive mutation off, 23 lives.
static void int_ga_scenario() {
	seed(202);
	CGA ga(5);
	ga.setAdaptiveMutation(false);
	ga.setPopType(TYPE_INTGAVALS);
	std::FILE* f = out_file("learn_intga.bin");
	for (int life = 0; life < 23; life++) {
		CIntGAValues* v = new CIntGAValues(RANDOM_LONG(0, 0x7fffffff));
		v->setFitness(RANDOM_FLOAT(0, 1));
		ga.addToPopulation(v);
	}
	ga.save(f);
	std::fclose(f);
}

// Two 21-bit CBitsGAValues: randomized (a's bits, then b's), crossed over, mutated 3 times, saved.
static void bits_scenario() {
	seed(303);
	// CBitsGAValues::random is inline in bot_ga.cpp, so randomize the CBits it is built on.
	CBits* ba = new CBits(21);
	CBits* bb = new CBits(21);
	ba->randomize();
	bb->randomize();
	CBitsGAValues a(ba), b(bb);
	a.crossOver(&b);
	for (int i = 0; i < 3; i++) {
		a.mutate();
		b.mutate();
	}
	a.setFitness(0.5f);
	std::FILE* f = out_file("learn_bits.bin");
	a.save(f);
	b.save(f);
	unsigned n = 0;
	a.convert(&n);
	put_u32(f, n);
	std::fclose(f);
}

// CPerceptron(4): 30 rounds of random inputs, execute, train towards (sum of inputs > 0).
static void perceptron_scenario() {
	seed(404);
	CPerceptron p(4);
	std::FILE* f = out_file("learn_perceptron.bin");
	for (int round = 0; round < 30; round++) {
		std::vector<ga_value> in;
		float sum = 0;
		for (int i = 0; i < 4; i++) {
			in.push_back(RANDOM_FLOAT(-1, 1));
			sum += in.back();
		}
		p.input(in);
		put_f32(f, p.execute());
		p.train(sum > 0 ? 1.0f : 0.0f);
	}
	p.save(f);
	std::fclose(f);
}

// NNGATrained(1, 3, 3, 2): 30 training rounds on 4 fixed sets; then its outputs on one input and
// the network as NN::save writes it.
static void nn_scenario() {
	seed(505);
	NNGATrained nn(1, 3, 3, 2);
	std::vector<CNNTrainSet> sets(4);
	for (int s = 0; s < 4; s++) {
		sets[s].inputs = {s * 0.25f, 1.0f - s * 0.25f, s % 2 ? 1.0f : 0.0f};
		sets[s].outputs = {s % 2 ? 1.0f : 0.0f, s < 2 ? 1.0f : 0.0f};
	}
	for (int round = 0; round < 30; round++)
		nn.train(sets);
	std::FILE* f = out_file("learn_nn.bin");
	std::vector<ga_value> out;
	nn.execute(out, {0.3f, 0.6f, 1.0f});
	put_u32(f, out.size());
	for (float o : out)
		put_f32(f, o);
	nn.save(f);
	std::fclose(f);
}

// CSom(4, 3, 2): 3 epochs over 5 fixed inputs, then every neuron's weights.
static void som_scenario() {
	seed(606);
	CSom som(4, 3, 2);
	std::vector<std::vector<ga_value>> inputs = {{0, 0}, {1, 1}, {0, 1}, {1, 0}, {0.5f, 0.5f}};
	som.input(inputs, 3);
	std::FILE* f = out_file("learn_som.bin");
	put_u32(f, som.epochs());
	// The neurons aren't exposed; the BMU of each input and its weights identify the map.
	for (const auto& in : inputs) {
		const CSomNeuron* n = som.getBMU(in);
		put_f32(f, n->getX());
		put_f32(f, n->getY());
		for (float w : n->weights())
			put_f32(f, w);
	}
	std::fclose(f);
}

// tsweaps.rld as upstream saves and loads it: a CGA() of 8-gene loadouts, saved, then loaded
// with upstream's chromosome size of 16 and saved again (design §5).
static void tsweaps_scenario() {
	seed(707);
	CGA ga;
	for (int life = 0; life < 20; life++) {
		CBotGAValues* v = new CBotGAValues();
		for (int i = 0; i < 8; i++)
			v->add(RANDOM_FLOAT(0, 1));
		v->setFitness(RANDOM_FLOAT(0, 1));
		ga.addToPopulation(v);
	}
	std::FILE* f = out_file("learn_tsweaps.rld");
	ga.save(f);
	std::fclose(f);

	CGA loaded;
	std::FILE* in = std::fopen((g_dir + "/learn_tsweaps.rld").c_str(), "rb");
	loaded.load(in, 16);
	std::fclose(in);
	std::FILE* f2 = out_file("learn_tsweaps_reloaded.rld");
	loaded.save(f2);
	std::fclose(f2);
}

int main(int argc, char** argv) {
	if (argc != 2) {
		std::fprintf(stderr, "usage: learn_harness <out dir>\n");
		return 2;
	}
	g_dir = argv[1];
	ga_scenario();
	int_ga_scenario();
	bits_scenario();
	perceptron_scenario();
	nn_scenario();
	som_scenario();
	tsweaps_scenario();
	return 0;
}
