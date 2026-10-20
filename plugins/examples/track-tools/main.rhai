fn on_event(input) {
    let output = #{
        ui: [#{
            type: "button",
            text: #{en: "Next", ru: "Далее"},
            command: #{type: "next"}
        }]
    };
    if input.settings.rename && input.settings.title != ""
        && input.snapshot != () && input.snapshot.playback != () {
        let patches = #{};
        patches[input.snapshot.playback.uri] = #{name: input.settings.title};
        output.track_overrides = patches;
    }
    output
}
