use super::super::DesignShaderGradientStop;
use super::*;

pub(super) fn shader_property_field_draft(
    value: &DesignShaderPropertyValue,
    field: ShaderPropertyEditorField,
) -> Option<(ShaderPropertyEditorInput, f64, String)> {
    let number = |value: f32, clamp| {
        (
            ShaderPropertyEditorInput::Number { clamp },
            f64::from(value),
            format_number(value),
        )
    };
    let unbounded = || None;
    let non_negative = || NumericClamp::new(Some(0.), None).ok();
    let normalized = || NumericClamp::new(Some(0.), Some(1.)).ok();
    match (value, field) {
        (DesignShaderPropertyValue::Text(value), ShaderPropertyEditorField::Text) => {
            Some((ShaderPropertyEditorInput::Text, 0., value.to_string()))
        }
        (DesignShaderPropertyValue::Number(value), ShaderPropertyEditorField::Number) => {
            Some(number(*value, unbounded()))
        }
        (DesignShaderPropertyValue::Color(color), ShaderPropertyEditorField::Color) => Some((
            ShaderPropertyEditorInput::Color,
            0.,
            color.hex().to_string(),
        )),
        (DesignShaderPropertyValue::Point(point), ShaderPropertyEditorField::PointX) => {
            Some(number(point.x, unbounded()))
        }
        (DesignShaderPropertyValue::Point(point), ShaderPropertyEditorField::PointY) => {
            Some(number(point.y, unbounded()))
        }
        (DesignShaderPropertyValue::Line { start, .. }, ShaderPropertyEditorField::LineStartX) => {
            Some(number(start.x, unbounded()))
        }
        (DesignShaderPropertyValue::Line { start, .. }, ShaderPropertyEditorField::LineStartY) => {
            Some(number(start.y, unbounded()))
        }
        (DesignShaderPropertyValue::Line { end, .. }, ShaderPropertyEditorField::LineEndX) => {
            Some(number(end.x, unbounded()))
        }
        (DesignShaderPropertyValue::Line { end, .. }, ShaderPropertyEditorField::LineEndY) => {
            Some(number(end.y, unbounded()))
        }
        (
            DesignShaderPropertyValue::Circle { center, .. },
            ShaderPropertyEditorField::CircleCenterX,
        ) => Some(number(center.x, unbounded())),
        (
            DesignShaderPropertyValue::Circle { center, .. },
            ShaderPropertyEditorField::CircleCenterY,
        ) => Some(number(center.y, unbounded())),
        (
            DesignShaderPropertyValue::Circle { radius, .. },
            ShaderPropertyEditorField::CircleRadius,
        ) => Some(number(*radius, non_negative())),
        (
            DesignShaderPropertyValue::CirclePoint { center, .. },
            ShaderPropertyEditorField::CirclePointCenterX,
        ) => Some(number(center.x, unbounded())),
        (
            DesignShaderPropertyValue::CirclePoint { center, .. },
            ShaderPropertyEditorField::CirclePointCenterY,
        ) => Some(number(center.y, unbounded())),
        (
            DesignShaderPropertyValue::CirclePoint { radius, .. },
            ShaderPropertyEditorField::CirclePointRadius,
        ) => Some(number(*radius, non_negative())),
        (
            DesignShaderPropertyValue::CirclePoint { angle, .. },
            ShaderPropertyEditorField::CirclePointAngle,
        ) => Some(number(*angle, unbounded())),
        (
            DesignShaderPropertyValue::ColorPoint { point, .. },
            ShaderPropertyEditorField::ColorPointX,
        ) => Some(number(point.x, unbounded())),
        (
            DesignShaderPropertyValue::ColorPoint { point, .. },
            ShaderPropertyEditorField::ColorPointY,
        ) => Some(number(point.y, unbounded())),
        (
            DesignShaderPropertyValue::ColorPoint { color, .. },
            ShaderPropertyEditorField::ColorPointColor,
        ) => Some((
            ShaderPropertyEditorInput::Color,
            0.,
            color.hex().to_string(),
        )),
        (
            DesignShaderPropertyValue::Gradient(stops),
            ShaderPropertyEditorField::GradientStopPosition(index),
        ) => Some(number(stops.get(index)?.position, normalized())),
        (
            DesignShaderPropertyValue::Gradient(stops),
            ShaderPropertyEditorField::GradientStopColor(index),
        ) => Some((
            ShaderPropertyEditorInput::Color,
            0.,
            stops.get(index)?.color.hex().to_string(),
        )),
        _ => None,
    }
}

pub(super) fn shader_property_value_from_draft(
    original: &DesignShaderPropertyValue,
    field: ShaderPropertyEditorField,
    input: ShaderPropertyEditorInput,
    draft: &str,
    base: f64,
) -> Result<DesignShaderPropertyValue, ()> {
    enum ParsedShaderPropertyDraft {
        Text(SharedString),
        Number(f32),
        Color(DesignColor),
    }

    let parsed = match input {
        ShaderPropertyEditorInput::Text => ParsedShaderPropertyDraft::Text(draft.to_owned().into()),
        ShaderPropertyEditorInput::Color => {
            ParsedShaderPropertyDraft::Color(parse_design_color(draft).ok_or(())?)
        }
        ShaderPropertyEditorInput::Number { clamp } => {
            let value = evaluate_numeric_expression(draft, base, clamp).map_err(|_| ())?;
            if value < f64::from(f32::MIN) || value > f64::from(f32::MAX) {
                return Err(());
            }
            ParsedShaderPropertyDraft::Number(value as f32)
        }
    };

    let mut candidate = original.clone();
    match (&mut candidate, field, parsed) {
        (
            DesignShaderPropertyValue::Text(value),
            ShaderPropertyEditorField::Text,
            ParsedShaderPropertyDraft::Text(next),
        ) => *value = next,
        (
            DesignShaderPropertyValue::Number(value),
            ShaderPropertyEditorField::Number,
            ParsedShaderPropertyDraft::Number(next),
        ) => *value = next,
        (
            DesignShaderPropertyValue::Color(color),
            ShaderPropertyEditorField::Color,
            ParsedShaderPropertyDraft::Color(next),
        ) => *color = next,
        (
            DesignShaderPropertyValue::Point(point),
            ShaderPropertyEditorField::PointX,
            ParsedShaderPropertyDraft::Number(next),
        ) => point.x = next,
        (
            DesignShaderPropertyValue::Point(point),
            ShaderPropertyEditorField::PointY,
            ParsedShaderPropertyDraft::Number(next),
        ) => point.y = next,
        (
            DesignShaderPropertyValue::Line { start, .. },
            ShaderPropertyEditorField::LineStartX,
            ParsedShaderPropertyDraft::Number(next),
        ) => start.x = next,
        (
            DesignShaderPropertyValue::Line { start, .. },
            ShaderPropertyEditorField::LineStartY,
            ParsedShaderPropertyDraft::Number(next),
        ) => start.y = next,
        (
            DesignShaderPropertyValue::Line { end, .. },
            ShaderPropertyEditorField::LineEndX,
            ParsedShaderPropertyDraft::Number(next),
        ) => end.x = next,
        (
            DesignShaderPropertyValue::Line { end, .. },
            ShaderPropertyEditorField::LineEndY,
            ParsedShaderPropertyDraft::Number(next),
        ) => end.y = next,
        (
            DesignShaderPropertyValue::Circle { center, .. },
            ShaderPropertyEditorField::CircleCenterX,
            ParsedShaderPropertyDraft::Number(next),
        ) => center.x = next,
        (
            DesignShaderPropertyValue::Circle { center, .. },
            ShaderPropertyEditorField::CircleCenterY,
            ParsedShaderPropertyDraft::Number(next),
        ) => center.y = next,
        (
            DesignShaderPropertyValue::Circle { radius, .. },
            ShaderPropertyEditorField::CircleRadius,
            ParsedShaderPropertyDraft::Number(next),
        ) => *radius = next,
        (
            DesignShaderPropertyValue::CirclePoint { center, .. },
            ShaderPropertyEditorField::CirclePointCenterX,
            ParsedShaderPropertyDraft::Number(next),
        ) => center.x = next,
        (
            DesignShaderPropertyValue::CirclePoint { center, .. },
            ShaderPropertyEditorField::CirclePointCenterY,
            ParsedShaderPropertyDraft::Number(next),
        ) => center.y = next,
        (
            DesignShaderPropertyValue::CirclePoint { radius, .. },
            ShaderPropertyEditorField::CirclePointRadius,
            ParsedShaderPropertyDraft::Number(next),
        ) => *radius = next,
        (
            DesignShaderPropertyValue::CirclePoint { angle, .. },
            ShaderPropertyEditorField::CirclePointAngle,
            ParsedShaderPropertyDraft::Number(next),
        ) => *angle = next,
        (
            DesignShaderPropertyValue::ColorPoint { point, .. },
            ShaderPropertyEditorField::ColorPointX,
            ParsedShaderPropertyDraft::Number(next),
        ) => point.x = next,
        (
            DesignShaderPropertyValue::ColorPoint { point, .. },
            ShaderPropertyEditorField::ColorPointY,
            ParsedShaderPropertyDraft::Number(next),
        ) => point.y = next,
        (
            DesignShaderPropertyValue::ColorPoint { color, .. },
            ShaderPropertyEditorField::ColorPointColor,
            ParsedShaderPropertyDraft::Color(next),
        ) => *color = next,
        (
            DesignShaderPropertyValue::Gradient(stops),
            ShaderPropertyEditorField::GradientStopPosition(index),
            ParsedShaderPropertyDraft::Number(next),
        ) => stops.get_mut(index).ok_or(())?.position = next,
        (
            DesignShaderPropertyValue::Gradient(stops),
            ShaderPropertyEditorField::GradientStopColor(index),
            ParsedShaderPropertyDraft::Color(next),
        ) => stops.get_mut(index).ok_or(())?.color = next,
        _ => return Err(()),
    }
    Ok(candidate)
}

pub(super) fn shader_property_variable_id(
    value: &DesignShaderPropertyValue,
    target: DesignShaderPropertyEditorTarget,
) -> Option<&SharedString> {
    match (value, target) {
        (
            DesignShaderPropertyValue::VariableAlias { variable_id },
            DesignShaderPropertyEditorTarget::Value,
        ) => Some(variable_id),
        (
            DesignShaderPropertyValue::ColorPoint { variable_id, .. },
            DesignShaderPropertyEditorTarget::ColorPointColor,
        ) => variable_id.as_ref(),
        (
            DesignShaderPropertyValue::Gradient(stops),
            DesignShaderPropertyEditorTarget::GradientStopColor(index),
        ) => stops.get(index)?.variable_id.as_ref(),
        _ => None,
    }
}

pub(super) fn shader_property_field_is_bound(
    value: &DesignShaderPropertyValue,
    field: ShaderPropertyEditorField,
) -> bool {
    match (value, field) {
        (
            DesignShaderPropertyValue::ColorPoint { variable_id, .. },
            ShaderPropertyEditorField::ColorPointColor,
        ) => variable_id.is_some(),
        (
            DesignShaderPropertyValue::Gradient(stops),
            ShaderPropertyEditorField::GradientStopColor(index),
        ) => stops
            .get(index)
            .is_some_and(|stop| stop.variable_id.is_some()),
        (DesignShaderPropertyValue::VariableAlias { .. }, _) => true,
        _ => false,
    }
}

pub(super) fn shader_gradient_with_added_stop(
    stops: &[DesignShaderGradientStop],
) -> Vec<DesignShaderGradientStop> {
    if stops.is_empty() {
        return vec![
            DesignShaderGradientStop::new(0., DesignColor::BLACK),
            DesignShaderGradientStop::new(1., DesignColor::WHITE),
        ];
    }
    if stops.len() == 1 {
        let mut next = stops.to_vec();
        let position = if stops[0].position <= 0.5 { 1. } else { 0. };
        next.push(DesignShaderGradientStop::new(position, stops[0].color));
        next.sort_by(|left, right| left.position.total_cmp(&right.position));
        return next;
    }

    let mut ordered = stops.to_vec();
    ordered.sort_by(|left, right| left.position.total_cmp(&right.position));
    let (left, right) = ordered
        .windows(2)
        .max_by(|left, right| {
            (left[1].position - left[0].position)
                .total_cmp(&(right[1].position - right[0].position))
        })
        .map(|pair| (&pair[0], &pair[1]))
        .expect("a gradient with at least two stops has an adjacent pair");
    let average = |left: u8, right: u8| ((u16::from(left) + u16::from(right)) / 2) as u8;
    let color = DesignColor::rgba(
        average(left.color.red, right.color.red),
        average(left.color.green, right.color.green),
        average(left.color.blue, right.color.blue),
        average(left.color.alpha, right.color.alpha),
    );
    ordered.push(DesignShaderGradientStop::new(
        (left.position + right.position) / 2.,
        color,
    ));
    ordered.sort_by(|left, right| left.position.total_cmp(&right.position));
    ordered
}
