/// Constante de tempo exponencial (~175ms para 98% da transição de scroll).
const TAU_SCROLL_SECS: f32 = 0.045;
/// Constante de tempo para entrada do dock (~155ms para 98%).
const TAU_OPEN_SECS: f32 = 0.040;
/// Constante de tempo para fade entre telas do Hub (~120ms para 98%).
const TAU_SCREEN_SECS: f32 = 0.030;
/// Duração total da micro-animação tátil de pressão do knob (~100ms).
const PRESS_PULSE_DURATION_SECS: f32 = 0.100;
/// Limiar de convergência abaixo do qual fazemos snap exato para repouso.
const EPSILON: f32 = 0.003;

/// Motor leve de animações exponenciais para o carrossel horizontal (60fps sob demanda).
///
/// Quando todas as propriedades atingem repouso (`is_active() == false`), o `Timer`
/// do Slint é desligado e o overlay consome 0% de CPU.
#[derive(Debug, Clone)]
pub struct CarouselAnimator {
    pub scroll_phase: f32,
    pub open_progress: f32,
    pub screen_progress: f32,
    /// Progresso restante da micro-animação de pressão (`1.0` → `0.0` em ~100ms; escala `1.0 - 0.02 * press_pulse`).
    pub press_pulse: f32,
    /// Fase contínua (`0.0..1.0`) para o indicador de carregamento quando `loading == true`.
    pub spinner_phase: f32,
    loading_active: bool,
}

impl Default for CarouselAnimator {
    fn default() -> Self {
        Self::new()
    }
}

impl CarouselAnimator {
    pub fn new() -> Self {
        Self {
            scroll_phase: 0.0,
            open_progress: 1.0,
            screen_progress: 1.0,
            press_pulse: 0.0,
            spinner_phase: 0.0,
            loading_active: false,
        }
    }

    pub fn reset(&mut self) {
        self.scroll_phase = 0.0;
        self.open_progress = 1.0;
        self.screen_progress = 1.0;
        self.press_pulse = 0.0;
        self.spinner_phase = 0.0;
        self.loading_active = false;
    }

    pub fn set_loading(&mut self, loading: bool, enabled: bool) {
        self.loading_active = loading && enabled;
        if !self.loading_active {
            self.spinner_phase = 0.0;
        }
    }

    pub fn trigger_open(&mut self, enabled: bool) {
        self.scroll_phase = 0.0;
        self.screen_progress = 1.0;
        self.press_pulse = 0.0;
        self.open_progress = if enabled { 0.0 } else { 1.0 };
    }

    pub fn trigger_scroll(&mut self, delta: i32, enabled: bool) {
        if !enabled || delta == 0 {
            self.scroll_phase = 0.0;
            return;
        }
        // Acumula giros rápidos em rajada para inércia contínua sem travar.
        self.scroll_phase = (self.scroll_phase + (delta as f32)).clamp(-3.0, 3.0);
    }

    pub fn trigger_screen_transition(&mut self, enabled: bool) {
        self.scroll_phase = 0.0;
        self.screen_progress = if enabled { 0.0 } else { 1.0 };
    }

    /// Dispara feedback tátil visual (escala 0.98× por ~100ms) ao pressionar o knob.
    pub fn trigger_press_pulse(&mut self, enabled: bool) {
        self.press_pulse = if enabled { 1.0 } else { 0.0 };
    }

    pub fn is_active(&self) -> bool {
        self.scroll_phase.abs() >= EPSILON
            || (1.0 - self.open_progress).abs() >= EPSILON
            || (1.0 - self.screen_progress).abs() >= EPSILON
            || self.press_pulse >= EPSILON
            || self.loading_active
    }

    /// Avança as animações por `dt_secs` (clampado para evitar saltos em frames atrasados).
    /// Retorna `true` se ainda há animação em andamento, ou `false` quando convergiu.
    pub fn step(&mut self, dt_secs: f32) -> bool {
        let dt = dt_secs.clamp(0.001, 0.05);

        if self.scroll_phase.abs() >= EPSILON {
            self.scroll_phase *= (-dt / TAU_SCROLL_SECS).exp();
            if self.scroll_phase.abs() < EPSILON {
                self.scroll_phase = 0.0;
            }
        } else {
            self.scroll_phase = 0.0;
        }

        if (1.0 - self.open_progress).abs() >= EPSILON {
            self.open_progress = 1.0 - (1.0 - self.open_progress) * (-dt / TAU_OPEN_SECS).exp();
            if (1.0 - self.open_progress).abs() < EPSILON {
                self.open_progress = 1.0;
            }
        } else {
            self.open_progress = 1.0;
        }

        if (1.0 - self.screen_progress).abs() >= EPSILON {
            self.screen_progress =
                1.0 - (1.0 - self.screen_progress) * (-dt / TAU_SCREEN_SECS).exp();
            if (1.0 - self.screen_progress).abs() < EPSILON {
                self.screen_progress = 1.0;
            }
        } else {
            self.screen_progress = 1.0;
        }

        if self.press_pulse >= EPSILON {
            self.press_pulse = (self.press_pulse - (dt / PRESS_PULSE_DURATION_SECS)).max(0.0);
            if self.press_pulse < EPSILON {
                self.press_pulse = 0.0;
            }
        } else {
            self.press_pulse = 0.0;
        }

        if self.loading_active {
            self.spinner_phase = (self.spinner_phase + dt * 1.6) % 1.0;
        }

        self.is_active()
    }
}

#[cfg(test)]
mod tests {
    use super::CarouselAnimator;

    #[test]
    fn scroll_accumulates_and_converges_smoothly_to_zero() {
        let mut anim = CarouselAnimator::new();
        assert!(!anim.is_active());

        anim.trigger_scroll(1, true);
        assert!(anim.is_active());
        assert!((anim.scroll_phase - 1.0).abs() < f32::EPSILON);

        // Giro rápido consecutivo acumula inércia
        anim.step(0.016);
        let mid = anim.scroll_phase;
        assert!(mid > 0.0 && mid < 1.0);
        anim.trigger_scroll(1, true);
        assert!(anim.scroll_phase > 1.0);

        // Converge completamente em menos de 400ms
        for _ in 0..30 {
            anim.step(0.016);
        }
        assert!(!anim.is_active());
        assert_eq!(anim.scroll_phase, 0.0);
    }

    #[test]
    fn press_pulse_decays_within_100ms() {
        let mut anim = CarouselAnimator::new();
        anim.trigger_press_pulse(true);
        assert!(anim.is_active());
        assert!((anim.press_pulse - 1.0).abs() < f32::EPSILON);

        for _ in 0..8 {
            anim.step(0.016);
        }
        assert_eq!(anim.press_pulse, 0.0);
        assert!(!anim.is_active());
    }

    #[test]
    fn disabled_animations_or_reduced_motion_snap_immediately() {
        let mut anim = CarouselAnimator::new();
        anim.trigger_open(false);
        anim.trigger_scroll(1, false);
        anim.trigger_screen_transition(false);
        anim.trigger_press_pulse(false);
        anim.set_loading(true, false);
        assert!(!anim.is_active());
        assert_eq!(anim.scroll_phase, 0.0);
        assert_eq!(anim.open_progress, 1.0);
        assert_eq!(anim.screen_progress, 1.0);
        assert_eq!(anim.press_pulse, 0.0);
    }
}
