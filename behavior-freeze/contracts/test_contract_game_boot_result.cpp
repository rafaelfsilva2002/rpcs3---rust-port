// Behavior-freeze contract: game_boot_result enum layout
//
// This test exists to freeze the ordinal values and total count of
// game_boot_result before a Rust rewrite. Any reorder or addition
// without updating this test is a behavior break that must be
// reviewed explicitly.
//
// Anchor: rpcs3/Emu/System.h:42-62

#include <gtest/gtest.h>

#include "Emu/System.h"

namespace behavior_freeze
{
	TEST(ContractGameBootResult, OrdinalValues)
	{
		// Freeze exact ordinal values. If you need to add a new value,
		// append it at the end and update EXPECTED_COUNT below.
		EXPECT_EQ(static_cast<u32>(game_boot_result::no_errors), 0u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::generic_error), 1u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::nothing_to_boot), 2u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::wrong_disc_location), 3u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::invalid_file_or_folder), 4u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::invalid_bdvd_folder), 5u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::install_failed), 6u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::decryption_error), 7u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::file_creation_error), 8u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::firmware_missing), 9u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::firmware_version), 10u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::unsupported_disc_type), 11u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::savestate_corrupted), 12u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::savestate_version_unsupported), 13u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::still_running), 14u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::already_added), 15u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::currently_restricted), 16u);
		EXPECT_EQ(static_cast<u32>(game_boot_result::database_config_missing), 17u);
	}

	TEST(ContractGameBootResult, IsErrorContract)
	{
		// Only no_errors is NOT an error. Everything else is.
		EXPECT_FALSE(is_error(game_boot_result::no_errors));
		EXPECT_TRUE(is_error(game_boot_result::generic_error));
		EXPECT_TRUE(is_error(game_boot_result::nothing_to_boot));
		EXPECT_TRUE(is_error(game_boot_result::decryption_error));
		EXPECT_TRUE(is_error(game_boot_result::firmware_missing));
		EXPECT_TRUE(is_error(game_boot_result::database_config_missing));
	}
}
