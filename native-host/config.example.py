# Copy to config.py (gitignored) and fill in your own repos and app.

# GitHub repo -> local main clone. Worktrees are created next to it as <clone>-<branch>.
REPOS = {
    "example/MyAndroidApp": "~/code/MyAndroidApp",
    "example/MyiOSApp": "~/code/MyiOSApp",
}

# Device runs. iOS: the scheme's Debug build; its app and bundle id come from the build settings.
IOS_SCHEME = "MyApp"
APP_ID = "com.example.myapp"
START_ACTIVITY = APP_ID + "/com.example.myapp.MainActivity"
PHONE_MODELS = ["model:Pixel_10", "model:Pixel_6"]  # preferred in this order, else any other phone
