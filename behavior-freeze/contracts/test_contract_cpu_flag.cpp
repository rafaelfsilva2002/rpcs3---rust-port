// Behavior-freeze contract: cpu_flag bitset
//
// cpu_flag is a bitset enum used to drive PPU/SPU scheduling via
// cpu_thread::check_state(). The ordinal position of each flag is
// serialized into savestates, so reordering silently breaks every
// savestate created with an older build.
//
// Anchor: rpcs3/Emu/CPU/CPUThread.h:14-38

#include <gtest/gtest.h>

#include "Emu/CPU/CPUThread.h"

namespace behavior_freeze
{
	TEST(ContractCpuFlag, FrozenOrdinalValues)
	{
		// Every variant of the C++ enum is locked here. Any reorder
		// silently breaks savestate serialization. Keep this in sync
		// with the Rust mirror in rust/rpcs3-emu-types/src/lib.rs
		// (search for CpuFlag::Stop).
		EXPECT_EQ(static_cast<u32>(cpu_flag::stop), 0u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::exit), 1u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::wait), 2u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::temp), 3u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::pause), 4u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::suspend), 5u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::ret), 6u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::again), 7u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::signal), 8u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::memory), 9u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::pending), 10u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::pending_recheck), 11u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::notify), 12u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::yield), 13u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::preempt), 14u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::req_exit), 15u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::dbg_global_pause), 16u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::dbg_pause), 17u);
		EXPECT_EQ(static_cast<u32>(cpu_flag::dbg_step), 18u);
	}
}
