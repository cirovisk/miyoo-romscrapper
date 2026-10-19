pub struct SystemMapping {
    pub id: &'static str,
    pub platforms: &'static str,
}

pub const ONION_SYSTEMS: &[SystemMapping] = &[
    SystemMapping {
        id: "AMIGA",
        platforms: "amiga",
    },
    SystemMapping {
        id: "CPC",
        platforms: "amstradcpc",
    },
    SystemMapping {
        id: "ARCADE",
        platforms: "mame-libretro",
    },
    SystemMapping {
        id: "ATARI",
        platforms: "atari2600",
    },
    SystemMapping {
        id: "FIFTYTWOHUNDRED",
        platforms: "atari5200",
    },
    SystemMapping {
        id: "SEVENTYEIGHTHUNDRED",
        platforms: "atari7800",
    },
    SystemMapping {
        id: "LYNX",
        platforms: "atarilynx",
    },
    SystemMapping {
        id: "WS",
        platforms: "wonderswan wonderswancolor",
    },
    SystemMapping {
        id: "COLECO",
        platforms: "coleco",
    },
    SystemMapping {
        id: "VECTREX",
        platforms: "vextrex",
    },
    SystemMapping {
        id: "INTELLIVISION",
        platforms: "intellivision",
    },
    SystemMapping {
        id: "MSX",
        platforms: "msx",
    },
    SystemMapping {
        id: "PCECD",
        platforms: "pcenginecd",
    },
    SystemMapping {
        id: "PCE",
        platforms: "pcengine",
    },
    SystemMapping {
        id: "FC",
        platforms: "nes",
    },
    SystemMapping {
        id: "GB",
        platforms: "gb gbc",
    },
    SystemMapping {
        id: "GBC",
        platforms: "gbc",
    },
    SystemMapping {
        id: "GBA",
        platforms: "gba",
    },
    SystemMapping {
        id: "POKE",
        platforms: "pokemini",
    },
    SystemMapping {
        id: "SFC",
        platforms: "snes",
    },
    SystemMapping {
        id: "VB",
        platforms: "virtualboy",
    },
    SystemMapping {
        id: "PICO",
        platforms: "pico8",
    },
    SystemMapping {
        id: "PORTS",
        platforms: "ports",
    },
    SystemMapping {
        id: "SCUMMVM",
        platforms: "scummvm",
    },
    SystemMapping {
        id: "THIRTYTWOX",
        platforms: "sega32x",
    },
    SystemMapping {
        id: "SEGACD",
        platforms: "segacd",
    },
    SystemMapping {
        id: "GG",
        platforms: "gamegear",
    },
    SystemMapping {
        id: "MD",
        platforms: "megadrive",
    },
    SystemMapping {
        id: "MS",
        platforms: "mastersystem",
    },
    SystemMapping {
        id: "SEGASGONE",
        platforms: "sg-1000",
    },
    SystemMapping {
        id: "ZXS",
        platforms: "zxspectrum",
    },
    SystemMapping {
        id: "NEOGEO",
        platforms: "neogeo",
    },
    SystemMapping {
        id: "NEOCD",
        platforms: "neogeocd",
    },
    SystemMapping {
        id: "NGP",
        platforms: "ngp ngpc",
    },
    SystemMapping {
        id: "PS",
        platforms: "psx",
    },
    SystemMapping {
        id: "VIDEOPAC",
        platforms: "videopac",
    },
];

pub fn mapping(system_id: &str) -> Option<&'static SystemMapping> {
    ONION_SYSTEMS.iter().find(|system| system.id == system_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn includes_onionscraper_system_mappings_and_multi_platform_systems() {
        assert_eq!(ONION_SYSTEMS.len(), 36);
        assert_eq!(mapping("GB").unwrap().platforms, "gb gbc");
        assert_eq!(
            mapping("WS").unwrap().platforms,
            "wonderswan wonderswancolor"
        );
        assert_eq!(mapping("NGP").unwrap().platforms, "ngp ngpc");
    }
}
