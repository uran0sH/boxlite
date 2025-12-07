
# ======================================
# 1. Foreground Colors (Text Color)
# Basic: 30-37 | Bright (Vibrant): 90-97
# ======================================
export BLACK='\033[0;30m'        # Black (basic)
export RED='\033[0;31m'          # Red (existing, retained)
export GREEN='\033[0;32m'        # Green (existing, retained)
export YELLOW='\033[1;33m'       # Yellow (existing, bold version, retained)
export BLUE='\033[0;34m'         # Blue (existing, retained)
export PURPLE='\033[0;35m'       # Purple (magenta)
export CYAN='\033[0;36m'         # Cyan (blue-green)
export WHITE='\033[0;37m'        # White (basic)

# Bright foreground colors (more vibrant for emphasis)
export BRIGHT_BLACK='\033[0;90m'  # Bright black (gray)
export BRIGHT_RED='\033[0;91m'    # Bright red
export BRIGHT_GREEN='\033[0;92m'  # Bright green
export BRIGHT_YELLOW='\033[0;93m' # Bright yellow (non-bold)
export BRIGHT_BLUE='\033[0;94m'   # Bright blue
export BRIGHT_PURPLE='\033[0;95m' # Bright purple
export BRIGHT_CYAN='\033[0;96m'   # Bright cyan
export BRIGHT_WHITE='\033[0;97m'  # Bright white

# ======================================
# 2. Background Colors (Text Background)
# Basic: 40-47 | Bright (Vibrant): 100-107
# ======================================
export BG_BLACK='\033[40m'        # Black background
export BG_RED='\033[41m'          # Red background
export BG_GREEN='\033[42m'        # Green background
export BG_YELLOW='\033[43m'       # Yellow background
export BG_BLUE='\033[44m'         # Blue background
export BG_PURPLE='\033[45m'       # Purple background
export BG_CYAN='\033[46m'         # Cyan background
export BG_WHITE='\033[47m'        # White background

# Bright background colors (more vibrant)
export BG_BRIGHT_BLACK='\033[100m' # Bright black background (gray)
export BG_BRIGHT_RED='\033[101m'   # Bright red background
export BG_BRIGHT_GREEN='\033[102m' # Bright green background
export BG_BRIGHT_YELLOW='\033[103m'# Bright yellow background
export BG_BRIGHT_BLUE='\033[104m'  # Bright blue background
export BG_BRIGHT_PURPLE='\033[105m'# Bright purple background
export BG_BRIGHT_CYAN='\033[106m'  # Bright cyan background
export BG_BRIGHT_WHITE='\033[107m'# Bright white background

# ======================================
# 3. Text Styles (Can combine with colors)
# ======================================
export BOLD='\033[1m'             # Bold (used in existing yellow, reusable)
export UNDERLINE='\033[4m'        # Underline
export BLINK='\033[5m'             # Blink (not supported by all terminals)
export REVERSE='\033[7m'          # Reverse video (swap text/background color)
export NC='\033[0m'               # No Color - reset all styles (existing, retained)