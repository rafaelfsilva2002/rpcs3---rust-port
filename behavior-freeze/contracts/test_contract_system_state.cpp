// Behavior-freeze contract: system_state enum layout
//
// Anchor: rpcs3/Emu/System.h:30-40

#include <gtest/gtest.h>

#include "Emu/System.h"

namespace behavior_freeze
{
	TEST(ContractSystemState, OrdinalValues)
	{
		EXPECT_EQ(static_cast<u32>(system_state::stopped), 0u);
		EXPECT_EQ(static_cast<u32>(system_state::loading), 1u);
		EXPECT_EQ(static_cast<u32>(system_state::stopping), 2u);
		EXPECT_EQ(static_cast<u32>(system_state::running), 3u);
		EXPECT_EQ(static_cast<u32>(system_state::paused), 4u);
		EXPECT_EQ(static_cast<u32>(system_state::frozen), 5u);
		EXPECT_EQ(static_cast<u32>(system_state::ready), 6u);
		EXPECT_EQ(static_cast<u32>(system_state::starting), 7u);
	}
}
