//! # Procedural Carbon of a Chemical
//!
//! Most reagents of a DNA synthesis run have a published cradle-to-gate
//! emission factor; a few do not. Cespi's compendium ("Procedural life
//! cycle inventory of chemical products at laboratory and pilot scale",
//! Green Chem. 26, 9554, 2024) collects how life-cycle practitioners fill
//! that gap, and [`kg_co2e_per_kg`] follows it:
//!
//! - the industrial route's stoichiometry at 95% yield (conversion 95%,
//!   selectivity 100%, the default of Hischier et al. 2005), each feedstock
//!   at its own factor;
//! - utilities at industrial averages, 1.2 MJ of electricity per kg of
//!   product (Althaus et al. 2007, from the Gendorf chemical park) and 1.8
//!   MJ of heat (Huber et al.), at the compendium's ecoinvent 3.10 factors
//!   (Table S14: 0.37 kg CO2e per kWh of European medium-voltage
//!   electricity, 0.41 per kWh of heat from steam);
//! - the unreacted 5% of the feed burned, its carbon leaving as CO2.
//!
//! The tests check it against eleven chemicals that one step makes from
//! feedstocks with published factors and that have a published factor of
//! their own. It lands a median 1.21 times the published figure, ten of
//! the eleven within a factor of 1.5; the feedstocks alone land a median
//! 0.89 times, so a chemical's real footprint sits between the two. The
//! compendium's other energy option, average heating and cooling for the
//! product's carbon count (its Table S3), ran a median 1.43 times over the
//! same eleven and is not used.

/// Yield the procedure assumes when a route's own is unknown.
pub const YIELD: f64 = 0.95;

/// kg CO2e per kg of product for utilities: 1.2 MJ of electricity at 0.37
/// kg CO2e/kWh and 1.8 MJ of heat at 0.41.
pub const UTILITIES_KG_CO2E_PER_KG: f64 = 1.2 / 3.6 * 0.37 + 1.8 / 3.6 * 0.41;

/// One feedstock of a reaction, per mole of product.
#[derive(Debug, Clone, Copy)]
pub struct Feed {
    pub moles: f64,
    /// g/mol.
    pub molar_mass: f64,
    pub carbon_atoms: u32,
    pub kg_co2e_per_kg: f64,
}

/// The feedstocks' share of [`kg_co2e_per_kg`]: what the reaction consumes
/// at [`YIELD`], without energy or waste.
pub fn feedstock_kg_co2e_per_kg(product_molar_mass: f64, feeds: &[Feed]) -> f64 {
    feeds.iter().map(|f| f.moles * f.molar_mass * f.kg_co2e_per_kg).sum::<f64>() / (YIELD * product_molar_mass)
}

/// Cradle-to-gate kg CO2e per kg of a product of molar mass
/// `product_molar_mass` made from `feeds`, by the procedure above.
pub fn kg_co2e_per_kg(product_molar_mass: f64, feeds: &[Feed]) -> f64 {
    let unreacted_carbon: f64 = feeds.iter().map(|f| f.moles * f64::from(f.carbon_atoms) * 12.011).sum::<f64>()
        * (1.0 / YIELD - 1.0)
        / product_molar_mass;
    // Burned at 99.99% efficiency (the compendium's eqn 7).
    let burned = unreacted_carbon * 44.01 / 12.011 * 0.9999;
    feedstock_kg_co2e_per_kg(product_molar_mass, feeds) + UTILITIES_KG_CO2E_PER_KG + burned
}

/// Feedstocks the three reagents of an array synthesis run without a
/// published factor are made from, with their factors: ammonia, ethylene
/// dichloride, ethylene oxide and NaOH from Idemat 2026RevB2; acetaldehyde,
/// formaldehyde, monoethanolamine and ethylene glycol from the ACS GCI
/// Pharmaceutical Roundtable's PMI-LCA tool (ecoinvent v2.2).
pub mod feeds {
    use super::Feed;

    const fn feed(moles: f64, molar_mass: f64, carbon_atoms: u32, kg_co2e_per_kg: f64) -> Feed {
        Feed { moles, molar_mass, carbon_atoms, kg_co2e_per_kg }
    }

    pub const fn ammonia(moles: f64) -> Feed {
        feed(moles, 17.031, 0, 2.0586)
    }
    pub const fn ethylene_dichloride(moles: f64) -> Feed {
        feed(moles, 98.954, 2, 1.1744)
    }
    pub const fn ethylene_oxide(moles: f64) -> Feed {
        feed(moles, 44.053, 2, 2.08)
    }
    pub const fn sodium_hydroxide(moles: f64) -> Feed {
        feed(moles, 39.997, 0, 0.7298)
    }
    pub const fn acetaldehyde(moles: f64) -> Feed {
        feed(moles, 44.053, 2, 1.3696)
    }
    pub const fn formaldehyde(moles: f64) -> Feed {
        feed(moles, 30.026, 1, 1.1056)
    }
    pub const fn monoethanolamine(moles: f64) -> Feed {
        feed(moles, 61.084, 2, 3.4386)
    }
    pub const fn ethylene_glycol(moles: f64) -> Feed {
        feed(moles, 62.068, 2, 1.5710)
    }
    pub const fn oxygen(moles: f64) -> Feed {
        feed(moles, 31.998, 0, 0.0)
    }
}

/// Ethylenediamine from ethylene dichloride, ammonia and NaOH (the
/// monoethanolamine route gives the same within 0.01).
pub fn ethylenediamine() -> f64 {
    use feeds::*;
    kg_co2e_per_kg(60.100, &[ethylene_dichloride(1.0), ammonia(2.0), sodium_hydroxide(2.0)])
}

/// Pyridine from acetaldehyde, formaldehyde and ammonia (Chichibabin).
pub fn pyridine() -> f64 {
    use feeds::*;
    kg_co2e_per_kg(79.102, &[acetaldehyde(2.0), formaldehyde(1.0), ammonia(1.0)])
}

/// Imidazole from glyoxal, formaldehyde and ammonia (Debus-Radziszewski),
/// the glyoxal itself from ethylene glycol by the same procedure.
pub fn imidazole() -> f64 {
    use feeds::*;
    let glyoxal = kg_co2e_per_kg(58.036, &[ethylene_glycol(1.0), oxygen(1.0)]);
    kg_co2e_per_kg(68.079, &[Feed { moles: 1.0, molar_mass: 58.036, carbon_atoms: 2, kg_co2e_per_kg: glyoxal }, formaldehyde(1.0), ammonia(2.0)])
}

#[cfg(test)]
mod tests {
    use super::feeds::*;
    use super::*;

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }

    #[test]
    fn the_procedure_brackets_eleven_published_factors() {
        let ethylene = |n| Feed { moles: n, molar_mass: 28.054, carbon_atoms: 2, kg_co2e_per_kg: 1.82 };
        let methanol = |n| Feed { moles: n, molar_mass: 32.042, carbon_atoms: 1, kg_co2e_per_kg: 0.9299 };
        let carbon_monoxide = Feed { moles: 1.0, molar_mass: 28.010, carbon_atoms: 1, kg_co2e_per_kg: 1.395 };
        let chlorine = Feed { moles: 1.0, molar_mass: 70.900, carbon_atoms: 0, kg_co2e_per_kg: 0.7628 };
        // (product, molar mass, feeds, published factor: Idemat 2026RevB2 or
        // the ACS GCI tool, the mean where both list it)
        let cases: Vec<(&str, f64, Vec<Feed>, f64)> = vec![
            ("ethanol", 46.069, vec![ethylene(1.0)], (1.3432 + 1.2481) / 2.0),
            ("acetic acid", 60.052, vec![methanol(1.0), carbon_monoxide], (1.6134 + 1.0908) / 2.0),
            ("diethanolamine", 105.137, vec![ethylene_oxide(2.0), ammonia(1.0)], 2.5251),
            ("monoethanolamine", 61.084, vec![ethylene_oxide(1.0), ammonia(1.0)], 3.4386),
            ("ethylene glycol", 62.068, vec![ethylene_oxide(1.0)], 1.5710),
            ("formaldehyde", 30.026, vec![methanol(1.0), oxygen(0.5)], (0.9950 + 1.1056) / 2.0),
            ("acetaldehyde", 44.053, vec![ethylene(1.0), oxygen(0.5)], 1.3696),
            ("methylamine", 31.058, vec![methanol(1.0), ammonia(1.0)], 2.9374),
            ("dimethylamine", 45.085, vec![methanol(2.0), ammonia(1.0)], 1.3072),
            ("ethylene dichloride", 98.954, vec![ethylene(1.0), chlorine], 1.1744),
            ("ethylene oxide", 44.053, vec![ethylene(1.0), oxygen(0.5)], 2.08),
        ];
        let full: Vec<f64> = cases.iter().map(|(_, m, f, published)| kg_co2e_per_kg(*m, f) / published).collect();
        let floor: Vec<f64> = cases.iter().map(|(_, m, f, published)| feedstock_kg_co2e_per_kg(*m, f) / published).collect();
        assert!((median(full.clone()) - 1.21).abs() < 0.02, "{full:?}");
        assert_eq!(full.iter().filter(|r| (0.67..=1.5).contains(*r)).count(), 10, "{full:?}");
        assert!((median(floor.clone()) - 0.89).abs() < 0.02, "{floor:?}");
        // Ethanol, for one: 1.17 of ethylene, 0.33 of utilities, 0.10 burned.
        assert!((kg_co2e_per_kg(46.069, &[ethylene(1.0)]) - 1.60).abs() < 0.01);
    }

    #[test]
    fn the_three_reagents_without_a_factor() {
        assert!((ethylenediamine() - 4.70).abs() < 0.01, "{}", ethylenediamine());
        let via_mea = kg_co2e_per_kg(60.100, &[monoethanolamine(1.0), ammonia(1.0)]);
        assert!((via_mea - ethylenediamine()).abs() < 0.02, "{via_mea}");
        assert!((pyridine() - 2.99).abs() < 0.01, "{}", pyridine());
        assert!((imidazole() - 3.98).abs() < 0.01, "{}", imidazole());
        // All above their feedstocks alone and below the Table S3 energy
        // option's figures (5.00, 3.16, 4.59).
        assert!(ethylenediamine() < 5.00 && pyridine() < 3.16 && imidazole() < 4.59);
    }
}
