use blueprint::Active;
use blueprint::Blueprint;
use blueprint::OLMC;
use blueprint::PinMode;
use chips::Chip;
use errors::at_line;
use errors::Error;
use errors::ErrorCode;
use gal;
use gal::Bounds;
use gal::GAL;
use gal::Mode;

pub fn build(blueprint: &Blueprint) -> Result<GAL, Error> {
    let mut gal = GAL::new(blueprint.chip);

    match gal.chip {
        Chip::GAL16V8 | Chip::GAL20V8 => build_galxv8(&mut gal, blueprint)?,
        Chip::GAL22V10 => build_gal22v10(&mut gal, blueprint)?,
        Chip::GAL20RA10 => build_gal20ra10(&mut gal, blueprint)?,
    }

    Ok(gal)
}

// Write out the signature.
fn set_sig(blueprint: &Blueprint, gal: &mut GAL) {
    // Signature has space for 8 bytes.
    for i in 0..usize::min(blueprint.sig.len(), 8) {
        let c = blueprint.sig[i];
        for j in 0..8 {
            gal.sig[i * 8 + j] = (c << j) & 0x80 != 0;
        }
    }
}

// Set the main expression and tristate.
fn set_core_eqns(gal: &mut GAL, blueprint: &Blueprint) -> Result<(), Error> {
    for (olmc, i) in blueprint.olmcs.iter().zip(0..) {
        let bounds = gal.chip.get_bounds(i);

        match &olmc.output {
            Some((_, term)) => {
                let bounds = tristate_adjust(gal, &olmc.output, &bounds);
                gal.add_term(&term, &bounds)?;
            }
            None => gal.add_term(&gal::false_term(0), &bounds)?,
        }

        if let Some(term) = &olmc.tri_con {
            gal.add_term(&term, &Bounds { row_offset: 0, max_row: 1, ..bounds })?;
        }
    }

    Ok(())
}

// Set ARST, APRST and CLK, only used by GAL20RA10
fn set_aux_eqns(gal: &mut GAL, blueprint: &Blueprint) -> Result<(), Error> {
    for (olmc, i) in blueprint.olmcs.iter().zip(0..) {
        let bounds = gal.chip.get_bounds(i);

        if olmc.output.is_some() {
            if let Some((PinMode::Registered, ref term)) = olmc.output {
                let arst_bounds = Bounds { row_offset: 2, max_row: 3, .. bounds };
                gal.add_term_opt(&olmc.arst, &arst_bounds)?;

                let aprst_bounds = Bounds { row_offset: 3, max_row: 4, .. bounds };
                gal.add_term_opt(&olmc.aprst, &aprst_bounds)?;

                if olmc.clock.is_none() {
                    return at_line(term.line_num, Err(ErrorCode::NoCLK));
                }
            }

            let clock_bounds = Bounds { row_offset: 1, max_row: 2, .. bounds };
            gal.add_term_opt(&olmc.clock, &clock_bounds)?;
        }
    }

    Ok(())
}

// Set the AR and SP equations, unique to the GAL22V10
fn set_arsp_eqns(gal: &mut GAL, blueprint: &Blueprint) -> Result<(), Error> {
    // AR
    let ar_bounds = Bounds { start_row: 0, max_row: 1, row_offset: 0 };
    gal.add_term_opt(&blueprint.ar, &ar_bounds)?;

    // SP
    let sp_bounds = Bounds { start_row: 131, max_row: 1, row_offset: 0 };
    gal.add_term_opt(&blueprint.sp, &sp_bounds)?;

    Ok(())
}

// We don't do anything with the PT bits in the GALxxV8s.
fn set_pts(gal: &mut GAL) {
    for bit in gal.pt.iter_mut() {
        *bit = true;
    }
}

// Adjust the bounds for the main term of there's a tristate enable
// term in the first row.
fn tristate_adjust(gal: &GAL, output: &Option<(PinMode, gal::Term)>, bounds: &Bounds) -> Bounds {
    match gal.chip {
        Chip::GAL16V8 | Chip::GAL20V8 => {
            let reg_out = if let Some((PinMode::Registered, _)) = output { true } else { false };
            if gal.get_mode() != Mode::Mode1 && !reg_out {
                Bounds { row_offset: 1, ..*bounds }
            } else {
                *bounds
            }
        }
        Chip::GAL22V10 => Bounds { row_offset: 1, ..*bounds },
        Chip::GAL20RA10 => Bounds { row_offset: 4, .. *bounds },
    }
}

// Check that you're not trying to use 20ra10-specific features
fn check_not_gal20ra10(blueprint: &Blueprint) -> Result<(), Error> {
    for olmc in blueprint.olmcs.iter() {
        if let Some(term) = &olmc.clock {
            return at_line(term.line_num, Err(ErrorCode::DisallowedCLK));
        }
        if let Some(term) = &olmc.arst {
            return at_line(term.line_num, Err(ErrorCode::DisallowedARST));
        }
        if let Some(term) = &olmc.aprst {
            return at_line(term.line_num, Err(ErrorCode::DisallowedAPRST));
        }
    }
    Ok(())
}

// Set the XOR bits for inverting outputs, if necessary.
fn set_xors(gal: &mut GAL, blueprint: &Blueprint) {
    let num_olmcs = blueprint.olmcs.len();
    for (olmc, i) in blueprint.olmcs.iter().zip(0..) {
        if olmc.output.is_some() && olmc.active == Active::High {
            gal.xor[num_olmcs - 1 - i] = true;
        }
    }
}

// Build the tristate control bits - set for inputs and tristated outputs.
fn build_tristate_flags(flags: &mut [bool], blueprint: &Blueprint, com_is_tri: bool) {
    let num_olmcs = blueprint.olmcs.len();
    for (olmc, i) in blueprint.olmcs.iter().zip(0..) {
        let is_tristate = match olmc.output {
            None => olmc.feedback,
            Some((PinMode::Tristate, _)) => true,
            Some((PinMode::Combinatorial, _)) => com_is_tri,
            Some((PinMode::Registered, _)) => false,
        };

        if is_tristate {
            flags[num_olmcs - 1 - i] = true;
        }
    }
}

////////////////////////////////////////////////////////////////////////
// GALxV8 analysis

pub fn get_mode_v8(olmcs: &[OLMC]) -> Mode {
    // If there's a registered pin, it's mode 3.
    for n in 0..8 {
        if let Some((PinMode::Registered, _)) = olmcs[n].output  {
            return Mode::Mode3;
        }
    }
    // If there's a tristate, it's mode 2.
    for n in 0..8 {
        if let Some((PinMode::Tristate, _)) = olmcs[n].output {
            return Mode::Mode2;
        }
    }
    // If we can't use mode 1, use mode 2.
    for n in 0..8 {
        // Some OLMCs cannot be configured as pure inputs in Mode 1.
        if olmcs[n].feedback && olmcs[n].output.is_none() {
            if n == 3 || n == 4 {
                return Mode::Mode2;
            }
        }
        // OLMC pins cannot be used as combinatorial feedback in Mode 1.
        if olmcs[n].feedback && olmcs[n].output.is_some() {
            return Mode::Mode2;
        }
    }
    // If there is still no mode defined, use mode 1.
    return Mode::Mode1;
}

////////////////////////////////////////////////////////////////////////
// Chip-specific GAL-building algorithms.
//

// TODO: Build order:
// X sig
// X ac0 and syn
// X fuses
// X xor
// ac1/s1
// X pt

fn build_galxv8(gal: &mut GAL, blueprint: &Blueprint) -> Result<(), Error> {
    check_not_gal20ra10(blueprint)?;
    set_sig(blueprint, gal);

    let mode = get_mode_v8(&blueprint.olmcs);
    // Sets AC0 and SYN
    gal.set_mode(mode);

    // Are we implementing combinatorial expressions as tristate?
    // Put combinatorial is only available in Mode 1.
    let com_is_tri = mode != Mode::Mode1;

    set_core_eqns(gal, blueprint)?;

    build_tristate_flags(&mut gal.ac1, blueprint, com_is_tri);

    set_xors(gal, blueprint);
    set_pts(gal);

    Ok(())
}

fn build_gal22v10(gal: &mut GAL, blueprint: &Blueprint) -> Result<(), Error> {
    check_not_gal20ra10(blueprint)?;
    set_sig(blueprint, gal);

    // TODO: Needs to be called before all the set_ands. Would be nice
    // to make independent.
    build_tristate_flags(&mut gal.s1, blueprint, true);

    set_core_eqns(gal, blueprint)?;
    set_arsp_eqns(gal, blueprint)?;
    set_xors(gal, blueprint);
    Ok(())
}

fn build_gal20ra10(gal: &mut GAL, blueprint: &Blueprint) -> Result<(), Error> {
    set_sig(blueprint, gal);
    set_core_eqns(gal, blueprint)?;
    set_aux_eqns(gal, blueprint)?;
    set_xors(gal, blueprint);
    Ok(())
}
