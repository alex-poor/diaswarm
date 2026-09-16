//! The page a clinician actually reads, for handing over by hand.
//!
//! **BECAUSE THE OTHER TWO ROUTES BOTH NEED SOMETHING FROM THE CLINIC.**
//! `$submit-cgm-bundle` needs them to run a FHIR server; a SMART Health Link
//! needs them to have a receiver. This needs nothing: it is one HTML file with
//! no scripts, no fonts and no network references, which prints to a sheet of
//! paper and opens on any phone. It is the route that works in a ten-minute
//! consultation today.
//!
//! **THE LAYOUT IS THE AGP CONVENTION, NOT AN INVENTION.** A stacked
//! time-in-range bar with the bands in clinical order, the headline metrics
//! beside it, and the data-sufficiency statement above everything — because a
//! summary of four days and a summary of ninety look identical otherwise.
//!
//! ⚠️ **IT IS THE SAME NUMBERS AS THE FHIR DOCUMENT, FROM THE SAME [`Agp`].**
//! Two renderings of one computation, so a clinician reading the page and a
//! system reading the bundle cannot be told different things.

use crate::agp::Agp;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Colours are the conventional AGP bands: red below, green in range, amber and
/// orange above. Chosen for print as much as for screen.
const BANDS: [(&str, &str); 5] = [
    ("very low", "#8c1c13"),
    ("low", "#c0392b"),
    ("in range", "#1e8449"),
    ("high", "#d68910"),
    ("very high", "#b9770e"),
];

pub fn html(agp: &Agp, patient: &str, window: &str) -> String {
    let pct = [
        agp.very_low_percent,
        agp.low_percent,
        agp.in_range_percent,
        agp.high_percent,
        agp.very_high_percent,
    ];

    let bar: String = BANDS
        .iter()
        .zip(pct)
        .filter(|(_, p)| *p > 0.0)
        .map(|((label, colour), p)| {
            format!(
                r#"<div class="seg" style="width:{p:.2}%;background:{colour}" title="{label} {p:.1}%"></div>"#
            )
        })
        .collect();

    let rows: String = BANDS
        .iter()
        .zip(pct)
        .map(|((label, colour), p)| {
            format!(
                r#"<tr><td><span class="sw" style="background:{colour}"></span>{label}</td><td class="n">{p:.1}%</td></tr>"#
            )
        })
        .collect();

    // **THE SUFFICIENCY STATEMENT GOES FIRST AND IS NOT SUBTLE.** A report that
    // quietly summarises four days is the failure this whole file exists to
    // avoid; the consensus asks for 14 days at 70% sensor active.
    let banner = match agp.insufficiency() {
        Some(why) => format!(
            r#"<p class="warn"><strong>Not a reliable estimate.</strong> {}. The international consensus asks for 14 days with the sensor active at least 70% of the time before these figures should guide a decision.</p>"#,
            esc(&why)
        ),
        None => r#"<p class="ok">Meets the consensus minimum for a reliable estimate: at least 14 days, sensor active at least 70% of the time.</p>"#.to_string(),
    };

    format!(
        r#"<!doctype html>
<meta charset="utf-8">
<title>CGM summary — {patient}</title>
<style>
 body{{font:14px/1.5 system-ui,sans-serif;max-width:46em;margin:2em auto;padding:0 1em;color:#111}}
 h1{{font-size:1.4em;margin:0 0 .2em}}
 .sub{{color:#555;margin:0 0 1.5em}}
 .warn{{background:#fdf0ef;border-left:4px solid #c0392b;padding:.8em 1em;margin:1.5em 0}}
 .ok{{background:#f0f7f2;border-left:4px solid #1e8449;padding:.8em 1em;margin:1.5em 0;color:#1e5631}}
 .bar{{display:flex;height:2.2em;border-radius:3px;overflow:hidden;margin:.5em 0 1em}}
 .seg{{height:100%}}
 table{{border-collapse:collapse;width:100%;margin:0 0 1.5em}}
 td,th{{padding:.35em .5em;border-bottom:1px solid #e3e3e3;text-align:left}}
 .n{{text-align:right;font-variant-numeric:tabular-nums}}
 .sw{{display:inline-block;width:.8em;height:.8em;border-radius:2px;margin-right:.5em;vertical-align:baseline}}
 .grid{{display:flex;gap:2em;flex-wrap:wrap;margin:0 0 1.5em}}
 .kpi strong{{display:block;font-size:1.6em;font-variant-numeric:tabular-nums}}
 .kpi span{{color:#555;font-size:.85em}}
 footer{{color:#666;font-size:.85em;border-top:1px solid #e3e3e3;padding-top:1em;margin-top:2em}}
 @media print{{body{{margin:0;max-width:none}} .warn,.ok{{border-left-width:3px}}}}
</style>
<h1>Continuous glucose monitoring summary</h1>
<p class="sub">{patient} · {window} · {readings} readings at about one every {cadence} seconds</p>
{banner}
<h2 style="font-size:1.05em">Time in ranges</h2>
<div class="bar">{bar}</div>
<table>{rows}</table>
<div class="grid">
 <div class="kpi"><strong>{mean:.0}</strong><span>mean glucose (mg/dL)</span></div>
 <div class="kpi"><strong>{gmi:.1}%</strong><span>glucose management indicator</span></div>
 <div class="kpi"><strong>{cv:.1}%</strong><span>coefficient of variation</span></div>
 <div class="kpi"><strong>{wear:.0}</strong><span>days of wear</span></div>
 <div class="kpi"><strong>{active:.1}%</strong><span>sensor active</span></div>
</div>
<footer>
 Bands are the 2019 international consensus on time in range: very low below 54 mg/dL,
 low 54–69, in range 70–180, high 181–250, very high above 250.
 GMI is Bergenstal's formula. Computed on the reader's own device from records the
 subject shared; nothing was uploaded to produce it.
</footer>
"#,
        patient = esc(patient),
        window = esc(window),
        readings = agp.readings,
        cadence = agp.cadence_seconds,
        mean = agp.mean_mgdl,
        gmi = agp.gmi_percent,
        cv = agp.cv_percent,
        wear = agp.days_of_wear,
        active = agp.sensor_active_percent,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agp(days: f64, active: f64) -> Agp {
        Agp {
            readings: 1000,
            days_of_wear: days,
            sensor_active_percent: active,
            mean_mgdl: 150.0,
            cv_percent: 32.4,
            gmi_percent: 6.898,
            very_low_percent: 0.5,
            low_percent: 2.5,
            in_range_percent: 72.0,
            high_percent: 20.0,
            very_high_percent: 5.0,
            cadence_seconds: 60,
            first_t: 0,
            last_t: 86_400_000,
        }
    }

    /// THE PAGE NEEDS NOTHING FROM THE NETWORK.
    ///
    /// A clinician opens this on a machine that may have no internet, or prints
    /// it. A remote font or script would make it degrade silently.
    #[test]
    fn the_report_is_self_contained() {
        let h = html(&agp(90.0, 96.0), "patient-1", "last 90 days");
        assert!(!h.contains("http://"), "the page reaches out to the network");
        assert!(!h.contains("https://"), "the page reaches out to the network");
        assert!(!h.contains("<script"), "the page needs scripting to render");
        assert!(h.contains("@media print"), "the page has no print rules");
    }

    /// AND IT LEADS WITH WHETHER THE NUMBERS CAN BE TRUSTED.
    #[test]
    fn insufficient_data_is_stated_before_the_numbers() {
        let short = html(&agp(4.0, 96.0), "p", "last 4 days");
        assert!(short.contains("Not a reliable estimate"), "a four-day summary read as reliable");
        assert!(
            short.find("Not a reliable estimate") < short.find("Time in ranges"),
            "the warning comes after the figures"
        );

        let good = html(&agp(90.0, 96.0), "p", "last 90 days");
        assert!(good.contains("Meets the consensus minimum"));
        assert!(!good.contains("Not a reliable estimate"));
    }

    /// THE BANDS ADD UP AND ARE ALL SHOWN.
    #[test]
    fn every_band_appears_with_its_percentage() {
        let h = html(&agp(90.0, 96.0), "p", "w");
        for label in ["very low", "low", "in range", "high", "very high"] {
            assert!(h.contains(label), "band {label} is missing");
        }
        assert!(h.contains("72.0%"), "the in-range figure is missing");
        assert!(h.contains("6.9%"), "GMI is missing");
    }

    #[test]
    fn a_hostile_patient_id_cannot_inject_markup() {
        let h = html(&agp(90.0, 96.0), "<script>alert(1)</script>", "w");
        assert!(!h.contains("<script>alert"), "markup was injected through the patient id");
        assert!(h.contains("&lt;script&gt;"));
    }
}
