#!/usr/bin/env python3
"""
BrowserBox Example - Secure Web Automation

Demonstrates BrowserBox API for isolated browser automation:
- Create browser instances (chromium, firefox, webkit)
- Get connection endpoints
- Connect with Puppeteer/Playwright
- Real-world use case: Cross-browser testing
"""

import asyncio
import boxlite


async def example_basic():
    """Example 1: Basic browser with default settings."""
    print("\n=== Example 1: Basic Browser (Chromium) ===")

    async with boxlite.BrowserBox() as browser:
        endpoint = browser.endpoint()
        print(f"✓ Browser ready: {endpoint}")
        print("\nConnect with Puppeteer:")
        print(f"  const browser = await puppeteer.connect({{ browserURL: '{endpoint}' }});")
        print("\nConnect with Playwright:")
        print(f"  const browser = await chromium.connect_over_cdp('{endpoint}');")

        await asyncio.sleep(30)


async def example_custom_browser():
    """Example 2: Custom browser configuration."""
    print("\n\n=== Example 2: Custom Configuration ===")

    # Firefox with custom resources
    opts = boxlite.BrowserBoxOptions(
        browser="firefox",
        memory=4096,
        cpu=2
    )

    async with boxlite.BrowserBox(opts) as browser:
        print(f"✓ Firefox ready: {browser.endpoint()}")
        print(f"  Memory: {opts.memory} MiB")
        print(f"  CPU: {opts.cpu} cores")

        await asyncio.sleep(30)


async def example_multiple_browsers():
    """Example 3: Multiple browsers for cross-browser testing."""
    print("\n\n=== Example 3: Cross-Browser Testing ===")

    # Create three browser types
    chromium_opts = boxlite.BrowserBoxOptions(browser="chromium")
    firefox_opts = boxlite.BrowserBoxOptions(browser="firefox")
    webkit_opts = boxlite.BrowserBoxOptions(browser="webkit")

    chromium = boxlite.BrowserBox(chromium_opts)
    firefox = boxlite.BrowserBox(firefox_opts)
    webkit = boxlite.BrowserBox(webkit_opts)

    async with chromium, firefox, webkit:
        print("✓ All browsers ready:")
        print(f"  Chromium: {chromium.endpoint()}")
        print(f"  Firefox:  {firefox.endpoint()}")
        print(f"  WebKit:   {webkit.endpoint()}")

        print("\n💡 Use Case: Cross-Browser Testing")
        print("  • Test your web app on all major engines")
        print("  • Connect to each endpoint with Puppeteer/Playwright")
        print("  • Run parallel test suites")
        print("  • Compare rendering and behavior")

        await asyncio.sleep(30)


async def main():
    """Run all examples."""
    print("BrowserBox Examples - Secure Web Automation")
    print("=" * 60)

    await example_basic()
    await example_custom_browser()
    await example_multiple_browsers()

    print("\n" + "=" * 60)
    print("✓ All examples completed!")
    print("\nKey Takeaways:")
    print("  • BrowserBox(options) - Create isolated browser")
    print("  • endpoint() - Get connection URL")
    print("  • Supports chromium, firefox, webkit")
    print("  • Connect with standard tools (Puppeteer/Playwright)")


if __name__ == "__main__":
    asyncio.run(main())
